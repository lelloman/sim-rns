//! Validated, optimistic source edits shared by the GTK node editor.
use super::*;

pub const SUPPORTED_TEMPLATES: &[&str] = &[
    "reticulum.python.backbone",
    "network.lan",
    "script.python",
    "script.bash",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSnapshot {
    pub project: Project,
    pub nodes: Vec<(String, ProjectNodeFile)>,
    pub scripts: Vec<(String, String)>,
    sources: BTreeMap<String, String>,
}

pub fn load(root: &Path) -> Result<EditorSnapshot, String> {
    let project = load_project(root)?;
    let mut sources = BTreeMap::new();
    for path in std::iter::once(PROJECT_FILE_NAME.to_string())
        .chain(project.file.includes.nodes.iter().cloned())
        .chain(project.file.includes.scripts.iter().cloned())
    {
        sources.insert(path.clone(), read_project_source(root, &path)?);
    }
    let nodes = project
        .file
        .includes
        .nodes
        .iter()
        .map(|p| {
            serde_json::from_str(&sources[p])
                .map(|node| (p.clone(), node))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scripts = project
        .file
        .includes
        .scripts
        .iter()
        .map(|p| (p.clone(), sources[p].clone()))
        .collect();
    Ok(EditorSnapshot {
        project,
        nodes,
        scripts,
        sources,
    })
}

#[derive(Clone)]
pub enum Edit {
    SaveNode {
        original_id: Option<String>,
        node: ProjectNodeFile,
    },
    RemoveNode {
        id: String,
    },
    SaveScript {
        path: String,
        contents: String,
    },
}

/// A prepared guest has persistent initialization. Changes require explicit consent
/// to archive that VM; the next Run prepares a fresh disk from the original bundle.
pub fn apply(expected: &EditorSnapshot, edit: Edit, fresh_guest: bool) -> Result<Project, String> {
    let root = &expected.project.root_path;
    let _lock = project_lock(root)?;
    let current = load(root)?;
    if current != *expected {
        return Err(
            "Project changed since this editor opened. Cancel and reopen it before saving.".into(),
        );
    }
    runtime::require_stopped_for_edit(&current.project)?;
    let mut project = current.project.clone();
    let mut nodes = current.nodes.clone();
    let mut writes = BTreeMap::new();
    match edit {
        Edit::SaveNode { original_id, node } => {
            if !SUPPORTED_TEMPLATES.contains(&node.template_id.as_str()) {
                return Err("Choose a supported guest template".into());
            }
            if node.assets.iter().any(|a| a.mode != AssetMode::Copy) {
                return Err("This guest supports copied assets only".into());
            }
            if node.template_id.starts_with("script.") && node.command_override.is_none() {
                return Err("Script nodes need script text or a command override".into());
            }
            if !project.file.startup.order.is_empty() {
                if !node.enabled {
                    project.file.startup.order.retain(|id| id != &node.id);
                } else if (original_id.is_none()
                    || current
                        .nodes
                        .iter()
                        .any(|(_, n)| n.id == node.id && !n.enabled))
                    && !project.file.startup.order.contains(&node.id)
                {
                    project.file.startup.order.push(node.id.clone());
                }
            }
            if let Some(id) = original_id {
                if id != node.id {
                    return Err("Existing node IDs cannot be renamed".into());
                }
                let (_, existing) = nodes
                    .iter_mut()
                    .find(|(_, n)| n.id == id)
                    .ok_or("Node no longer exists")?;
                *existing = node;
            } else {
                // IDs are validated below; never derive a filesystem path from unvalidated input.
                let ids = project_recipe(&current.project)?
                    .elements
                    .into_iter()
                    .map(|e| e.id)
                    .collect();
                let (path, _) =
                    unique_project_entry_path(root, PROJECT_NODES_DIR, "node", ".node.json", &ids);
                project.file.includes.nodes.push(path.clone());
                nodes.push((path, node));
            }
        }
        Edit::RemoveNode { id } => {
            let index = nodes
                .iter()
                .position(|(_, n)| n.id == id)
                .ok_or("Node no longer exists")?;
            let (path, _) = nodes.remove(index);
            project.file.includes.nodes.retain(|p| p != &path);
            project.file.startup.order.retain(|entry| entry != &id);
            for (_, node) in &mut nodes {
                node.attachments.retain(|network| network != &id);
            }
            // Unreferenced source files are retained so removal never destroys custom files.
        }
        Edit::SaveScript { path, contents } => {
            if !project.file.includes.scripts.contains(&path) {
                return Err("Unknown project script".into());
            }
            if contents.len() > 1024 * 1024 || contents.contains('\0') {
                return Err("Script must be UTF-8 text of at most 1 MiB without NUL bytes".into());
            }
            writes.insert(path, contents);
        }
    }
    let recipe = recipe_with_nodes(&project, nodes.iter().map(|(_, n)| n.clone()).collect())?;
    if recipe.elements.len() > 200 {
        return Err("Guest supports at most 200 elements".into());
    }
    for (_, node) in &nodes {
        if node.template_id == "network.lan" && !node.attachments.is_empty() {
            return Err("A LAN cannot be attached to another LAN".into());
        }
        for network in &node.attachments {
            if node.enabled
                && !nodes
                    .iter()
                    .any(|(_, n)| &n.id == network && n.enabled && n.template_id == "network.lan")
            {
                return Err(format!("{} needs an enabled LAN: {network}", node.id));
            }
        }
    }
    for (path, node) in nodes {
        let contents = serde_json::to_string_pretty(&node).map_err(|e| e.to_string())?;
        if current
            .nodes
            .iter()
            .find(|(p, _)| p == &path)
            .map(|(_, n)| n)
            != Some(&node)
        {
            writes.insert(path, contents);
        }
    }
    writes.retain(|path, text| current.sources.get(path) != Some(text));
    if writes.is_empty() && project.file == current.project.file {
        return Ok(project);
    }
    project.file.updated_at_unix_ms = unix_time_ms()?;
    writes.insert(
        PROJECT_FILE_NAME.into(),
        serde_json::to_string_pretty(&project.file).map_err(|e| e.to_string())?,
    );
    // Resolve existing paths and verify parents of new sources before changing anything.
    let mut destinations = BTreeMap::new();
    for (path, contents) in &writes {
        if contents.len() > 1024 * 1024 {
            return Err("Project source files must be at most 1 MiB".into());
        }
        let destination = if current.sources.contains_key(path) {
            project_source_path(root, path)?
        } else {
            let destination = root.join(path);
            let parent =
                std::fs::canonicalize(destination.parent().unwrap()).map_err(|e| e.to_string())?;
            if !parent.starts_with(root) || destination.symlink_metadata().is_ok() {
                return Err("Unsafe new node path".into());
            }
            destination
        };
        destinations.insert(path.clone(), destination);
    }
    let layout = QemuRuntime::default().layout(&project);
    let backup = if layout.disk_image_path.exists() {
        if !fresh_guest {
            return Err("This project has a prepared guest. Select Start a fresh guest to preserve its VM in a backup and apply the new configuration on next Run.".into());
        }
        let canonical = std::fs::canonicalize(&layout.vm_dir).map_err(|e| e.to_string())?;
        if canonical != layout.vm_dir || !canonical.starts_with(root) {
            return Err("Unsafe VM directory".into());
        }
        let backup = layout
            .runtime_dir
            .join(format!("previous-vm-{}", unix_time_ms()?));
        if backup.exists() {
            return Err("VM backup already exists; retry saving".into());
        }
        std::fs::rename(&layout.vm_dir, &backup).map_err(|e| e.to_string())?;
        Some(backup)
    } else {
        None
    };
    let mut changed = Vec::new();
    let result = (|| {
        for (path, contents) in &writes {
            changed.push(path.clone());
            atomic_write(&destinations[path], contents.as_bytes())?;
        }
        load_project(root)
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for path in changed.into_iter().rev() {
            let restore = if let Some(original) = current.sources.get(&path) {
                atomic_write(&destinations[&path], original.as_bytes())
            } else {
                std::fs::remove_file(&destinations[&path]).map_err(|e| e.to_string())
            };
            if let Err(e) = restore {
                failures.push(e);
            }
        }
        if let Some(backup) = backup {
            if let Err(e) = std::fs::rename(backup, &layout.vm_dir) {
                failures.push(e.to_string());
            }
        }
        return Err(format!(
            "Save failed: {error}; rollback errors: {failures:?}"
        ));
    }
    result
}
