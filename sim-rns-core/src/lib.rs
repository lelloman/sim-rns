use persistence::{atomic_write, project_lock};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

mod guest;
pub mod persistence;
mod qmp;
pub mod runtime;
pub use runtime::{
    FileBackedRuntime, NodeRuntimeState, ProjectRuntime, QemuRuntime, RuntimeBackendState,
    RuntimeCommand, RuntimeCommandOutcome, RuntimeError, RuntimeEvent, RuntimeSnapshot,
    RuntimeStatus, RuntimeTopologyOverlay, RuntimeVmAssets, RuntimeVmLayout, RuntimeVmState,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recipe {
    pub metadata: RecipeMetadata,
    pub vm: VmSetup,
    pub templates: Vec<Template>,
    pub elements: Vec<Element>,
    pub topology: Topology,
    pub startup: StartupPlan,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecipeMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmSetup {
    pub base_image: String,
    pub os_family: String,
    pub ram_mb: u32,
    pub cpu_cores: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub label: String,
    pub category: TemplateCategory,
    pub extends: Option<String>,
    pub description: String,
    pub runtime: RuntimeSpec,
    pub defaults: TemplateDefaults,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateCategory {
    Reticulum,
    Network,
    Script,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub family: RuntimeFamily,
    pub image_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFamily {
    Binary,
    Python,
    Bash,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TemplateDefaults {
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub restart_policy: RestartPolicy,
    pub resources: ResourceLimits,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Element {
    pub id: String,
    pub template_id: String,
    pub enabled: bool,
    pub env: BTreeMap<String, String>,
    pub assets: Vec<AssetSeed>,
    pub restart_policy: Option<RestartPolicy>,
    pub resources: Option<ResourceLimits>,
    pub command_override: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetSeed {
    pub source: String,
    pub destination: String,
    pub mode: AssetMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetMode {
    Copy,
    Template,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResourceLimits {
    pub memory_mb: u32,
    pub cpu_weight: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Topology {
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Attachment {
    pub element_id: String,
    pub network_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StartupPlan {
    pub order: Vec<String>,
}

pub const PROJECT_FILE_NAME: &str = "sim-rns.project.json";
pub const PROJECT_CONFIGS_DIR: &str = "configs";
pub const PROJECT_NODES_DIR: &str = "nodes";
pub const PROJECT_SCRIPTS_DIR: &str = "scripts";
pub const PROJECT_ASSETS_DIR: &str = "assets";
const PROJECT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTransport {
    Local,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectHandle {
    pub transport: ProjectTransport,
    pub path: String,
    pub display_name: String,
}

impl ProjectHandle {
    pub fn for_local_dir(path: impl AsRef<Path>) -> Result<Self, String> {
        let normalized = normalize_local_project_path(path.as_ref())?;
        let display_name = normalized
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| normalized.as_os_str().to_str().unwrap_or("project"))
            .to_string();
        Ok(Self {
            transport: ProjectTransport::Local,
            path: normalized.to_string_lossy().into_owned(),
            display_name,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self)
            .map_err(|error| format!("failed to serialize project handle: {error}"))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes)
            .map_err(|error| format!("failed to deserialize project handle: {error}"))
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectIncludes {
    #[serde(default)]
    pub configs: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(default)]
    pub assets: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectNodeFile {
    pub id: String,
    pub template_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub assets: Vec<AssetSeed>,
    #[serde(default)]
    pub restart_policy: Option<RestartPolicy>,
    #[serde(default)]
    pub resources: Option<ResourceLimits>,
    #[serde(default)]
    pub command_override: Option<Vec<String>>,
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectFile {
    pub schema_version: u32,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub vm: VmSetup,
    #[serde(default)]
    pub includes: ProjectIncludes,
    #[serde(default)]
    pub startup: StartupPlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub root_path: PathBuf,
    pub file: ProjectFile,
}

impl Project {
    pub fn handle(&self) -> ProjectHandle {
        ProjectHandle {
            transport: ProjectTransport::Local,
            path: self.root_path.to_string_lossy().into_owned(),
            display_name: self.file.name.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LauncherConfig {
    #[serde(default)]
    pub last_guest_bundle: Option<String>,
    #[serde(default)]
    pub recent_projects: Vec<ProjectHandle>,
}

impl LauncherConfig {
    pub fn remember_project(&mut self, handle: ProjectHandle) {
        self.recent_projects.retain(|existing| {
            existing.path != handle.path || existing.transport != handle.transport
        });
        self.recent_projects.insert(0, handle);
        if self.recent_projects.len() > 10 {
            self.recent_projects.truncate(10);
        }
    }
}

type ProjectOpenCallback = dyn Fn(ProjectHandle) -> Result<(), String> + 'static;
type ProjectCloseCallback = dyn Fn() -> Result<(), String> + 'static;

thread_local! {
    static PROJECT_OPENER: RefCell<Option<Box<ProjectOpenCallback>>> = RefCell::new(None);
    static PROJECT_CLOSER: RefCell<Option<Box<ProjectCloseCallback>>> = RefCell::new(None);
    static ACTIVE_PROJECT_HANDLE: RefCell<Option<ProjectHandle>> = const { RefCell::new(None) };
}

pub fn install_project_opener<F>(opener: F)
where
    F: Fn(ProjectHandle) -> Result<(), String> + 'static,
{
    PROJECT_OPENER.with(|callback| {
        callback.replace(Some(Box::new(opener)));
    });
}

pub fn install_project_closer<F>(closer: F)
where
    F: Fn() -> Result<(), String> + 'static,
{
    PROJECT_CLOSER.with(|callback| {
        callback.replace(Some(Box::new(closer)));
    });
}

pub fn open_project(handle: ProjectHandle) -> Result<(), String> {
    PROJECT_OPENER.with(|callback| {
        let callback = callback.borrow();
        let opener = callback
            .as_ref()
            .ok_or_else(|| "project opener is not installed".to_string())?;
        opener(handle)
    })
}

pub fn close_project() -> Result<(), String> {
    PROJECT_CLOSER.with(|callback| {
        let callback = callback.borrow();
        let closer = callback
            .as_ref()
            .ok_or_else(|| "project closer is not installed".to_string())?;
        closer()
    })
}

pub fn set_active_project_handle(handle: Option<ProjectHandle>) {
    ACTIVE_PROJECT_HANDLE.with(|current| {
        current.replace(handle);
    });
}

pub fn current_project_handle() -> Option<ProjectHandle> {
    ACTIVE_PROJECT_HANDLE.with(|current| current.borrow().clone())
}

pub fn current_project() -> Result<Project, String> {
    let handle =
        current_project_handle().ok_or_else(|| "no active project is selected".to_string())?;
    load_project(handle.path)
}

pub fn normalize_local_project_path(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {}: {error}", path.display()))
}

pub fn project_file_path(root_path: impl AsRef<Path>) -> PathBuf {
    root_path.as_ref().join(PROJECT_FILE_NAME)
}

pub fn is_project_dir(path: impl AsRef<Path>) -> bool {
    let root = path.as_ref();
    root.is_dir() && project_file_path(root).is_file()
}

pub fn load_project(path: impl AsRef<Path>) -> Result<Project, String> {
    let root_path = normalize_local_project_path(path.as_ref())?;
    let file_path = project_file_path(&root_path);
    let payload = std::fs::read_to_string(&file_path)
        .map_err(|error| format!("failed to read {}: {error}", file_path.display()))?;
    let file: ProjectFile = serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", file_path.display()))?;
    if file.schema_version != PROJECT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported project schema version {} in {}",
            file.schema_version,
            file_path.display()
        ));
    }
    let project = Project { root_path, file };
    project_recipe(&project)?;
    Ok(project)
}

/// Read a bounded UTF-8 source file, excluding private runtime and VCS metadata.
pub fn read_project_source(root: &Path, relative: &str) -> Result<String, String> {
    let path = project_source_path(root, relative)?;
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

fn project_source_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = resolve_project_relative_path(root, relative)?;
    let canonical_root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    if path
        .strip_prefix(canonical_root)
        .map_err(|e| e.to_string())?
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    {
        return Err("private project metadata is not a source file".into());
    }
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("project source files must be at most 1 MiB".into());
    }
    Ok(path)
}

/// Atomically replace an existing text source, restoring it if recipe validation fails.
pub fn write_project_source(
    root: &Path,
    relative: &str,
    contents: &str,
) -> Result<Project, String> {
    if contents.len() > 1024 * 1024 {
        return Err("project source files must be at most 1 MiB".into());
    }
    let _lock = project_lock(root)?;
    runtime::require_stopped_for_edit(&load_project(root)?)?;
    let path = project_source_path(root, relative)?;
    let original = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    atomic_write(&path, contents.as_bytes())?;
    match load_project(root) {
        Ok(project) => Ok(project),
        Err(error) => {
            atomic_write(&path, original.as_bytes())
                .map_err(|restore| format!("{error}; rollback failed: {restore}"))?;
            Err(format!("edit rejected and original restored: {error}"))
        }
    }
}

/// Validate the manifest and every image before creating a project or importing a VM.
pub fn validate_guest_bundle(path: impl AsRef<Path>) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(path.as_ref())
        .map_err(|e| format!("Cannot open guest bundle: {e}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("guest.json"))
            .map_err(|e| format!("Choose a guest bundle directory containing guest.json: {e}"))?,
    )
    .map_err(|e| format!("Invalid guest.json: {e}"))?;
    if manifest["version"] != 1 {
        return Err("Unsupported guest bundle version (expected 1)".into());
    }
    for field in ["kernel", "initrd", "disk"] {
        let relative = manifest[field]
            .as_str()
            .ok_or_else(|| format!("Guest bundle is missing {field}"))?;
        let file = resolve_project_relative_path(&root, relative)?;
        let metadata = std::fs::File::open(&file)
            .and_then(|f| f.metadata())
            .map_err(|e| format!("Cannot read {field}: {e}"))?;
        if metadata.len() == 0 {
            return Err(format!("Guest bundle {field} file is empty"));
        }
    }
    let disk = resolve_project_relative_path(&root, manifest["disk"].as_str().unwrap())?;
    let output = std::process::Command::new("qemu-img")
        .args(["info", "--output=json"])
        .arg(disk)
        .output()
        .map_err(|e| format!("Cannot validate the guest disk; install qemu-img: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Invalid guest disk: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let info: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    if info["virtual-size"].as_u64().unwrap_or(0) == 0 {
        return Err("Guest disk is empty".into());
    }
    Ok(root)
}

/// Create the supported two-node configuration using a validated guest bundle.
pub fn create_project_with_bundle(
    path: impl AsRef<Path>,
    name: &str,
    bundle: impl AsRef<Path>,
) -> Result<Project, String> {
    let bundle = validate_guest_bundle(bundle)?;
    let mut project = create_project(path, name)?;
    project.file.vm.base_image = bundle.to_string_lossy().into_owned();
    write_project_file(&project)?;
    load_project(&project.root_path)
}

/// Compatibility helper for the original demo command.
pub fn create_demo_project(
    path: impl AsRef<Path>,
    bundle: impl AsRef<Path>,
) -> Result<Project, String> {
    create_project_with_bundle(path, "Reticulum Guest Demo", bundle)
}

/// Low-level source scaffold; user-facing creation must use create_project_with_bundle.
pub fn create_project(root_path: impl AsRef<Path>, name: &str) -> Result<Project, String> {
    let requested_root = root_path.as_ref();
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err("project name cannot be empty".to_string());
    }

    if requested_root.exists() {
        if !requested_root.is_dir() {
            return Err(format!("{} is not a directory", requested_root.display()));
        }
        let mut entries = std::fs::read_dir(requested_root)
            .map_err(|error| format!("failed to inspect {}: {error}", requested_root.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} is not empty; choose an empty directory for a new project",
                requested_root.display()
            ));
        }
    } else {
        std::fs::create_dir_all(requested_root)
            .map_err(|error| format!("failed to create {}: {error}", requested_root.display()))?;
    }

    let root_path = normalize_local_project_path(requested_root)?;
    let timestamp = unix_time_ms()?;
    let project_id = slugify_project_name(trimmed_name);
    let file = ProjectFile {
        schema_version: PROJECT_SCHEMA_VERSION,
        project_id: project_id.clone(),
        name: trimmed_name.to_string(),
        description: "Two Python Reticulum nodes connected over a virtual LAN".to_string(),
        created_at_unix_ms: timestamp,
        updated_at_unix_ms: timestamp,
        vm: VmSetup {
            base_image: "sim-rns-guest-v1".to_string(),
            os_family: "debian".to_string(),
            ram_mb: 512,
            cpu_cores: 1,
        },
        includes: ProjectIncludes {
            configs: vec!["configs/backbone-a.toml".to_string()],
            nodes: vec![
                "nodes/lan-main.node.json".to_string(),
                "nodes/backbone-a.node.json".to_string(),
                "nodes/phone-a.node.json".to_string(),
            ],
            scripts: vec!["scripts/traffic_seed.py".to_string()],
            assets: Vec::new(),
        },
        startup: StartupPlan {
            order: vec![
                "lan-main".to_string(),
                "backbone-a".to_string(),
                "phone-a".to_string(),
                "traffic-seed".to_string(),
            ],
        },
    };

    for dir_name in [
        PROJECT_CONFIGS_DIR,
        PROJECT_NODES_DIR,
        PROJECT_SCRIPTS_DIR,
        PROJECT_ASSETS_DIR,
    ] {
        std::fs::create_dir_all(root_path.join(dir_name)).map_err(|error| {
            format!(
                "failed to create {} in {}: {error}",
                dir_name,
                root_path.display()
            )
        })?;
    }

    write_project_scaffold_files(&root_path, &project_id)?;

    let file_path = project_file_path(&root_path);
    let payload = serde_json::to_string_pretty(&file)
        .map_err(|error| format!("failed to serialize project file: {error}"))?;
    atomic_write(&file_path, payload.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", file_path.display()))?;

    Ok(Project { root_path, file })
}

fn write_project_file(project: &Project) -> Result<(), String> {
    let file_path = project_file_path(&project.root_path);
    let payload = serde_json::to_string_pretty(&project.file)
        .map_err(|error| format!("failed to serialize project file: {error}"))?;
    atomic_write(&file_path, payload.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", file_path.display()))
}

fn write_project_scaffold_files(root_path: &Path, project_id: &str) -> Result<(), String> {
    write_pretty_json(
        root_path.join(PROJECT_NODES_DIR).join("lan-main.node.json"),
        &ProjectNodeFile {
            id: "lan-main".to_string(),
            template_id: "network.lan".to_string(),
            enabled: true,
            env: BTreeMap::new(),
            assets: Vec::new(),
            restart_policy: Some(RestartPolicy::Never),
            resources: None,
            command_override: None,
            attachments: Vec::new(),
        },
    )?;
    write_pretty_json(
        root_path
            .join(PROJECT_NODES_DIR)
            .join("backbone-a.node.json"),
        &ProjectNodeFile {
            id: "backbone-a".to_string(),
            template_id: "reticulum.python.backbone".to_string(),
            enabled: true,
            env: BTreeMap::from([("RNS_INSTANCE".to_string(), "backbone-a".to_string())]),
            assets: vec![AssetSeed {
                source: "configs/backbone-a.toml".to_string(),
                destination: "config/config.toml".to_string(),
                mode: AssetMode::Copy,
            }],
            restart_policy: None,
            resources: None,
            command_override: None,
            attachments: vec!["lan-main".to_string()],
        },
    )?;
    write_pretty_json(
        root_path.join(PROJECT_NODES_DIR).join("phone-a.node.json"),
        &ProjectNodeFile {
            id: "phone-a".to_string(),
            template_id: "reticulum.python.backbone".to_string(),
            enabled: true,
            env: BTreeMap::from([("RNS_INSTANCE".to_string(), "phone-a".to_string())]),
            assets: Vec::new(),
            restart_policy: Some(RestartPolicy::OnFailure),
            resources: Some(ResourceLimits {
                memory_mb: 256,
                cpu_weight: 100,
            }),
            command_override: None,
            attachments: vec!["lan-main".to_string()],
        },
    )?;
    write_file(
        root_path.join(PROJECT_CONFIGS_DIR).join("backbone-a.toml"),
        "[reticulum]\ninstance = \"backbone-a\"\nannounce_interval = 30\n",
    )?;
    write_file(
        root_path.join(PROJECT_SCRIPTS_DIR).join("traffic_seed.py"),
        &format!("print(\"Sim RNS traffic seed for project {project_id}\")\n"),
    )?;
    Ok(())
}

fn write_pretty_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    atomic_write(&path, payload.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn write_file(path: PathBuf, contents: &str) -> Result<(), String> {
    atomic_write(&path, contents.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn project_templates() -> Vec<Template> {
    let mut templates = base_templates();
    templates.push(Template {
        id: "custom.phone".to_string(),
        label: "Phone Persona".to_string(),
        category: TemplateCategory::Reticulum,
        extends: Some("lxmf.python.client".to_string()),
        description: "Custom phone-oriented LXMF profile.".to_string(),
        runtime: RuntimeSpec {
            family: RuntimeFamily::Python,
            image_features: vec!["python-reticulum".to_string()],
        },
        defaults: TemplateDefaults {
            command: vec![
                "python3".to_string(),
                "/opt/sim-rns/lxmf_phone.py".to_string(),
            ],
            env: BTreeMap::from([
                ("SIM_PERSONA".to_string(), "phone".to_string()),
                ("SIM_SLEEP_PROFILE".to_string(), "intermittent".to_string()),
            ]),
            restart_policy: RestartPolicy::OnFailure,
            resources: ResourceLimits {
                memory_mb: 192,
                cpu_weight: 80,
            },
        },
    });
    templates
}

fn resolve_project_relative_path(root_path: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative_path.trim().is_empty() {
        return Err("project include path cannot be empty".to_string());
    }
    if relative.is_absolute() {
        return Err(format!(
            "{relative_path} must be relative to the project root"
        ));
    }
    if relative
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "{relative_path} cannot escape the project root with parent segments"
        ));
    }
    let resolved = root_path.join(relative);
    if !resolved.exists() {
        return Err(format!(
            "{} referenced by the project file does not exist",
            resolved.display()
        ));
    }
    let canonical = std::fs::canonicalize(&resolved).map_err(|e| e.to_string())?;
    let root = std::fs::canonicalize(root_path).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(format!(
            "{relative_path} must resolve to a file inside the project"
        ));
    }
    Ok(canonical)
}

fn load_node_file(root_path: &Path, relative_path: &str) -> Result<ProjectNodeFile, String> {
    let path = resolve_project_relative_path(root_path, relative_path)?;
    let payload = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn infer_script_template_id(script_path: &str) -> Result<&'static str, String> {
    if script_path.ends_with(".py") {
        Ok("script.python")
    } else if script_path.ends_with(".sh") {
        Ok("script.bash")
    } else {
        Err(format!(
            "{script_path} uses an unsupported script extension; use .py or .sh"
        ))
    }
}

fn build_script_element(root_path: &Path, relative_path: &str) -> Result<Element, String> {
    let resolved = resolve_project_relative_path(root_path, relative_path)?;
    if !resolved.is_file() {
        return Err(format!("{} is not a file", resolved.display()));
    }
    let template_id = infer_script_template_id(relative_path)?;
    let script_id = Path::new(relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(slugify_project_name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("failed to derive a script id from {relative_path}"))?;
    let command_override = if template_id == "script.python" {
        vec!["python3".to_string(), relative_path.to_string()]
    } else {
        vec!["bash".to_string(), relative_path.to_string()]
    };

    Ok(Element {
        id: script_id,
        template_id: template_id.to_string(),
        enabled: true,
        env: BTreeMap::new(),
        assets: vec![AssetSeed {
            source: relative_path.to_string(),
            destination: relative_path.to_string(),
            mode: AssetMode::Copy,
        }],
        restart_policy: Some(RestartPolicy::Never),
        resources: Some(ResourceLimits {
            memory_mb: 64,
            cpu_weight: 20,
        }),
        command_override: Some(command_override),
    })
}

pub fn project_recipe(project: &Project) -> Result<Recipe, String> {
    for path in project
        .file
        .includes
        .configs
        .iter()
        .chain(&project.file.includes.assets)
    {
        resolve_project_relative_path(&project.root_path, path)?;
    }
    let mut elements = Vec::new();
    let mut attachments = Vec::new();

    for node_path in &project.file.includes.nodes {
        let node = load_node_file(&project.root_path, node_path)?;
        attachments.extend(node.attachments.iter().map(|network_id| Attachment {
            element_id: node.id.clone(),
            network_id: network_id.clone(),
        }));
        elements.push(Element {
            id: node.id,
            template_id: node.template_id,
            enabled: node.enabled,
            env: node.env,
            assets: node.assets,
            restart_policy: node.restart_policy,
            resources: node.resources,
            command_override: node.command_override,
        });
    }

    for script_path in &project.file.includes.scripts {
        elements.push(build_script_element(&project.root_path, script_path)?);
    }

    let startup = if project.file.startup.order.is_empty() {
        StartupPlan {
            order: elements
                .iter()
                .filter(|element| element.enabled)
                .map(|element| element.id.clone())
                .collect(),
        }
    } else {
        project.file.startup.clone()
    };

    let recipe = Recipe {
        metadata: RecipeMetadata {
            id: project.file.project_id.clone(),
            name: project.file.name.clone(),
            description: project.file.description.clone(),
        },
        vm: project.file.vm.clone(),
        templates: project_templates(),
        elements,
        topology: Topology { attachments },
        startup,
    };
    validate_recipe(&recipe)?;
    for element in &recipe.elements {
        for asset in &element.assets {
            resolve_project_relative_path(&project.root_path, &asset.source)?;
            validate_relative_destination(&asset.destination)?;
        }
    }
    Ok(recipe)
}

fn validate_relative_destination(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.contains('\0')
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("invalid asset destination {value:?}"));
    }
    Ok(())
}

pub fn validate_recipe(recipe: &Recipe) -> Result<(), String> {
    fn identifier(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 64
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }
    if !identifier(&recipe.metadata.id) || recipe.metadata.name.trim().is_empty() {
        return Err("project needs a valid ID and a nonempty name".into());
    }
    if recipe.vm.ram_mb == 0 || recipe.vm.cpu_cores == 0 || recipe.vm.base_image.trim().is_empty() {
        return Err("VM requires a base image, positive RAM, and at least one CPU".into());
    }
    let templates: BTreeMap<_, _> = recipe
        .templates
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    if templates.len() != recipe.templates.len() {
        return Err("duplicate template IDs".into());
    }
    let mut elements = BTreeMap::new();
    for element in &recipe.elements {
        if !identifier(&element.id) || elements.insert(element.id.as_str(), element).is_some() {
            return Err(format!("invalid or duplicate element ID: {}", element.id));
        }
        let template = templates.get(element.template_id.as_str()).ok_or_else(|| {
            format!(
                "unknown template {} for {}",
                element.template_id, element.id
            )
        })?;
        let limits = element
            .resources
            .as_ref()
            .unwrap_or(&template.defaults.resources);
        if limits.cpu_weight > 10000 {
            return Err(format!("{} has a CPU weight above 10000", element.id));
        }
        let command = element
            .command_override
            .as_ref()
            .unwrap_or(&template.defaults.command);
        if command.is_empty()
            || command[0].trim().is_empty()
            || command.iter().any(|arg| arg.contains('\0'))
        {
            return Err(format!("{} has an invalid command", element.id));
        }
        if element
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err(format!(
                "{} has an invalid environment variable",
                element.id
            ));
        }
    }
    let mut startup = BTreeSet::new();
    for id in &recipe.startup.order {
        if !elements.contains_key(id.as_str()) || !startup.insert(id) {
            return Err(format!("unknown or duplicate startup element {id}"));
        }
    }
    let mut links = BTreeSet::new();
    for link in &recipe.topology.attachments {
        let network = elements
            .get(link.network_id.as_str())
            .ok_or_else(|| format!("unknown network {}", link.network_id))?;
        if !elements.contains_key(link.element_id.as_str())
            || link.element_id == link.network_id
            || templates[network.template_id.as_str()].category != TemplateCategory::Network
            || !links.insert(link)
        {
            return Err(format!(
                "invalid topology attachment {} -> {}",
                link.element_id, link.network_id
            ));
        }
    }
    Ok(())
}

fn unique_project_entry_path(
    root_path: &Path,
    dir_name: &str,
    stem: &str,
    suffix: &str,
    used_ids: &BTreeSet<String>,
) -> (String, PathBuf) {
    for index in 1.. {
        let file_name = format!("{stem}-{index}{suffix}");
        let relative = format!("{dir_name}/{file_name}");
        let absolute = root_path.join(&relative);
        if !absolute.exists() && !used_ids.contains(&format!("{stem}-{index}")) {
            return (relative, absolute);
        }
    }
    unreachable!("the filesystem should eventually provide a free path")
}

pub fn add_script_include(project_root: impl AsRef<Path>) -> Result<(Project, String), String> {
    let root = normalize_local_project_path(project_root.as_ref())?;
    let _lock = project_lock(&root)?;
    let mut project = load_project(&root)?;
    let used_ids = project_recipe(&project)?
        .elements
        .into_iter()
        .map(|e| e.id)
        .collect();
    let (relative_path, absolute_path) = unique_project_entry_path(
        &project.root_path,
        PROJECT_SCRIPTS_DIR,
        "script",
        ".py",
        &used_ids,
    );
    let script_id = Path::new(&relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(slugify_project_name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("failed to derive a script id from {relative_path}"))?;

    write_file(
        absolute_path,
        &format!("print(\"Sim RNS script stub: {script_id}\")\n"),
    )?;
    project.file.includes.scripts.push(relative_path.clone());
    if !project.file.startup.order.is_empty()
        && !project
            .file
            .startup
            .order
            .iter()
            .any(|entry| entry == &script_id)
    {
        project.file.startup.order.push(script_id);
    }
    project.file.updated_at_unix_ms = unix_time_ms()?;
    write_project_file(&project)?;
    Ok((project, relative_path))
}

pub fn add_node_include(project_root: impl AsRef<Path>) -> Result<(Project, String), String> {
    let root = normalize_local_project_path(project_root.as_ref())?;
    let _lock = project_lock(&root)?;
    let mut project = load_project(&root)?;
    let used_ids = project_recipe(&project)?
        .elements
        .into_iter()
        .map(|e| e.id)
        .collect();
    let (relative_path, absolute_path) = unique_project_entry_path(
        &project.root_path,
        PROJECT_NODES_DIR,
        "node",
        ".node.json",
        &used_ids,
    );
    let node_id = Path::new(&relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.trim_end_matches(".node"))
        .map(slugify_project_name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("failed to derive a node id from {relative_path}"))?;

    write_pretty_json(
        absolute_path,
        &ProjectNodeFile {
            id: node_id.clone(),
            template_id: "reticulum.python.backbone".to_string(),
            enabled: true,
            env: BTreeMap::from([("RNS_INSTANCE".to_string(), node_id.clone())]),
            assets: Vec::new(),
            restart_policy: Some(RestartPolicy::OnFailure),
            resources: Some(ResourceLimits {
                memory_mb: 256,
                cpu_weight: 100,
            }),
            command_override: None,
            attachments: project_recipe(&project)?
                .elements
                .iter()
                .find(|e| e.enabled && e.template_id == "network.lan")
                .map(|e| vec![e.id.clone()])
                .unwrap_or_default(),
        },
    )?;
    project.file.includes.nodes.push(relative_path.clone());
    if !project.file.startup.order.is_empty()
        && !project
            .file
            .startup
            .order
            .iter()
            .any(|entry| entry == &node_id)
    {
        project.file.startup.order.push(node_id);
    }
    project.file.updated_at_unix_ms = unix_time_ms()?;
    write_project_file(&project)?;
    Ok((project, relative_path))
}

fn unix_time_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| format!("system clock error: {error}"))
}

fn slugify_project_name(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

pub fn base_templates() -> Vec<Template> {
    vec![
        Template {
            id: "rns.rs.backbone".to_string(),
            label: "RNS Rust Backbone".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Rust backbone node powered by rnsd.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["rns-rs".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["rnsd".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 256,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "lxmf.rs.client".to_string(),
            label: "LXMF Rust Client".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Rust LXMF client actor.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["lxmf-rs".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["lxmfd".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "reticulum.python.backbone".to_string(),
            label: "Python Reticulum Backbone".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Python Reticulum backbone runtime.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec![
                    "python3".to_string(),
                    "/opt/sim-rns/reticulum_backbone.py".to_string(),
                ],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 256,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "lxmf.python.client".to_string(),
            label: "Python LXMF Client".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Python LXMF client runtime.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec![
                    "python3".to_string(),
                    "/opt/sim-rns/lxmf_client.py".to_string(),
                ],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "network.lan".to_string(),
            label: "LAN Segment".to_string(),
            category: TemplateCategory::Network,
            extends: None,
            description: "Shared network segment for attaching elements.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["iproute2".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["/usr/bin/env".to_string(), "true".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 32,
                    cpu_weight: 10,
                },
            },
        },
        Template {
            id: "script.python".to_string(),
            label: "Python Script".to_string(),
            category: TemplateCategory::Script,
            extends: None,
            description: "Generic Python script runner.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python3".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["python3".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 128,
                    cpu_weight: 50,
                },
            },
        },
        Template {
            id: "script.bash".to_string(),
            label: "Bash Script".to_string(),
            category: TemplateCategory::Script,
            extends: None,
            description: "Generic Bash script runner.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Bash,
                image_features: vec!["bash".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["bash".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 64,
                    cpu_weight: 30,
                },
            },
        },
    ]
}

pub fn sample_recipe() -> Recipe {
    let templates = {
        let mut templates = base_templates();
        templates.push(Template {
            id: "custom.phone".to_string(),
            label: "Phone Persona".to_string(),
            category: TemplateCategory::Reticulum,
            extends: Some("lxmf.python.client".to_string()),
            description: "Custom phone-oriented LXMF profile.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec![
                    "python3".to_string(),
                    "/opt/sim-rns/lxmf_phone.py".to_string(),
                ],
                env: BTreeMap::from([
                    ("SIM_PERSONA".to_string(), "phone".to_string()),
                    ("SIM_SLEEP_PROFILE".to_string(), "intermittent".to_string()),
                ]),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 80,
                },
            },
        });
        templates
    };

    Recipe {
        metadata: RecipeMetadata {
            id: "mesh-lab-01".to_string(),
            name: "Mesh Lab 01".to_string(),
            description: "Starter recipe for a VM-backed Reticulum experiment.".to_string(),
        },
        vm: VmSetup {
            base_image: "sim-rns-guest-v1".to_string(),
            os_family: "debian".to_string(),
            ram_mb: 4096,
            cpu_cores: 4,
        },
        templates,
        elements: vec![
            Element {
                id: "backbone-a".to_string(),
                template_id: "rns.rs.backbone".to_string(),
                enabled: true,
                env: BTreeMap::from([("RNS_INSTANCE".to_string(), "backbone-a".to_string())]),
                assets: vec![AssetSeed {
                    source: "assets/backbone-a/config.toml".to_string(),
                    destination: "config/config.toml".to_string(),
                    mode: AssetMode::Template,
                }],
                restart_policy: None,
                resources: None,
                command_override: None,
            },
            Element {
                id: "phone-a".to_string(),
                template_id: "custom.phone".to_string(),
                enabled: true,
                env: BTreeMap::from([("LXMF_DISPLAY_NAME".to_string(), "phone-a".to_string())]),
                assets: vec![],
                restart_policy: Some(RestartPolicy::OnFailure),
                resources: Some(ResourceLimits {
                    memory_mb: 160,
                    cpu_weight: 70,
                }),
                command_override: None,
            },
            Element {
                id: "lan-main".to_string(),
                template_id: "network.lan".to_string(),
                enabled: true,
                env: BTreeMap::new(),
                assets: vec![],
                restart_policy: Some(RestartPolicy::Never),
                resources: None,
                command_override: None,
            },
            Element {
                id: "traffic-seed".to_string(),
                template_id: "script.python".to_string(),
                enabled: false,
                env: BTreeMap::from([("SIM_TRIGGER".to_string(), "initial-burst".to_string())]),
                assets: vec![AssetSeed {
                    source: "scripts/traffic_seed.py".to_string(),
                    destination: "scripts/traffic_seed.py".to_string(),
                    mode: AssetMode::Copy,
                }],
                restart_policy: Some(RestartPolicy::Never),
                resources: Some(ResourceLimits {
                    memory_mb: 64,
                    cpu_weight: 20,
                }),
                command_override: Some(vec![
                    "python3".to_string(),
                    "scripts/traffic_seed.py".to_string(),
                ]),
            },
        ],
        topology: Topology {
            attachments: vec![
                Attachment {
                    element_id: "backbone-a".to_string(),
                    network_id: "lan-main".to_string(),
                },
                Attachment {
                    element_id: "phone-a".to_string(),
                    network_id: "lan-main".to_string(),
                },
            ],
        },
        startup: StartupPlan {
            order: vec![
                "lan-main".to_string(),
                "backbone-a".to_string(),
                "phone-a".to_string(),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_node_include, add_script_include, create_project, is_project_dir, load_project,
        normalize_local_project_path, project_file_path, project_recipe, LauncherConfig,
        ProjectHandle, ProjectTransport, PROJECT_ASSETS_DIR, PROJECT_CONFIGS_DIR,
        PROJECT_FILE_NAME, PROJECT_NODES_DIR, PROJECT_SCRIPTS_DIR,
    };

    fn unique_test_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn project_handle_round_trips_through_bytes() {
        let handle = ProjectHandle {
            transport: ProjectTransport::Local,
            path: "/tmp/mesh-lab".to_string(),
            display_name: "mesh-lab".to_string(),
        };

        let encoded = handle.to_bytes().expect("project handle should serialize");
        let decoded =
            ProjectHandle::from_bytes(&encoded).expect("project handle should deserialize");

        assert_eq!(decoded, handle);
    }

    #[test]
    fn recent_projects_are_deduplicated_and_trimmed() {
        let mut config = LauncherConfig::default();

        for index in 0..12 {
            config.remember_project(ProjectHandle {
                transport: ProjectTransport::Local,
                path: format!("/tmp/project-{index}"),
                display_name: format!("project-{index}"),
            });
        }

        config.remember_project(ProjectHandle {
            transport: ProjectTransport::Local,
            path: "/tmp/project-5".to_string(),
            display_name: "project-5".to_string(),
        });

        assert_eq!(config.recent_projects.len(), 10);
        assert_eq!(config.recent_projects[0].path, "/tmp/project-5");
        assert_eq!(config.recent_projects[1].path, "/tmp/project-11");
        assert!(!config
            .recent_projects
            .iter()
            .any(|project| project.path == "/tmp/project-0"));
    }

    #[test]
    fn local_project_path_validation_requires_existing_directory() {
        let temp_dir = unique_test_dir("sim-rns-core-path-test");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");

        let normalized =
            normalize_local_project_path(&temp_dir).expect("directory should validate");
        assert!(normalized.is_absolute());

        let missing = temp_dir.join("missing");
        assert!(normalize_local_project_path(&missing).is_err());

        std::fs::remove_dir_all(&temp_dir).expect("temp dir should be removed");
    }

    #[test]
    fn create_and_load_project_round_trip() {
        let root = unique_test_dir("sim-rns-core-project-test");
        let created = create_project(&root, "Mesh Lab").expect("project should be created");

        assert_eq!(created.file.name, "Mesh Lab");
        assert!(project_file_path(&root).ends_with(PROJECT_FILE_NAME));
        assert!(is_project_dir(&root));
        assert!(root.join(PROJECT_CONFIGS_DIR).is_dir());
        assert!(root.join(PROJECT_NODES_DIR).is_dir());
        assert!(root.join(PROJECT_SCRIPTS_DIR).is_dir());
        assert!(root.join(PROJECT_ASSETS_DIR).is_dir());
        assert!(root
            .join(PROJECT_NODES_DIR)
            .join("lan-main.node.json")
            .is_file());
        assert!(root
            .join(PROJECT_NODES_DIR)
            .join("backbone-a.node.json")
            .is_file());
        assert!(root
            .join(PROJECT_NODES_DIR)
            .join("phone-a.node.json")
            .is_file());
        assert!(root
            .join(PROJECT_SCRIPTS_DIR)
            .join("traffic_seed.py")
            .is_file());

        let loaded = load_project(&root).expect("project should load");
        assert_eq!(loaded.file.name, "Mesh Lab");
        assert_eq!(loaded.file.project_id, "mesh-lab");
        assert_eq!(loaded.handle().display_name, "Mesh Lab");

        std::fs::remove_dir_all(&root).expect("temp dir should be removed");
    }

    #[test]
    fn create_project_rejects_non_empty_directory() {
        let root = unique_test_dir("sim-rns-core-project-nonempty");
        std::fs::create_dir_all(&root).expect("temp dir should be created");
        std::fs::write(root.join("notes.txt"), "occupied").expect("temp file should be written");

        let error = create_project(&root, "Busy Project").expect_err("creation should fail");
        assert!(error.contains("not empty"));

        std::fs::remove_dir_all(&root).expect("temp dir should be removed");
    }

    #[test]
    fn project_recipe_is_derived_from_root_file_and_imports() {
        let root = unique_test_dir("sim-rns-core-project-recipe");
        let project = create_project(&root, "Recipe Test").expect("project should be created");

        let recipe = project_recipe(&project).expect("recipe should derive from project imports");
        assert_eq!(recipe.metadata.id, "recipe-test");
        assert_eq!(recipe.metadata.name, "Recipe Test");
        assert_eq!(recipe.vm.base_image, "sim-rns-guest-v1");
        assert_eq!(recipe.elements.len(), 4);
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id == "lan-main"));
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id == "backbone-a"));
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id == "phone-a"));
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id == "traffic-seed" && element.template_id == "script.python"));
        assert_eq!(recipe.topology.attachments.len(), 2);
        assert_eq!(recipe.startup.order[0], "lan-main");

        std::fs::remove_dir_all(&root).expect("temp dir should be removed");
    }

    #[test]
    fn add_include_actions_extend_the_project_tree() {
        let root = unique_test_dir("sim-rns-core-project-actions");
        create_project(&root, "Action Test").expect("project should be created");

        let (project_with_script, script_path) =
            add_script_include(&root).expect("script include should be added");
        assert!(root.join(&script_path).is_file());
        assert!(project_with_script
            .file
            .includes
            .scripts
            .iter()
            .any(|path| path == &script_path));

        let (project_with_node, node_path) =
            add_node_include(&root).expect("node include should be added");
        assert!(root.join(&node_path).is_file());
        assert!(project_with_node
            .file
            .includes
            .nodes
            .iter()
            .any(|path| path == &node_path));

        let recipe = project_recipe(&project_with_node).expect("recipe should still derive");
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id.starts_with("script-")));
        assert!(recipe
            .elements
            .iter()
            .any(|element| element.id.starts_with("node-")));

        std::fs::remove_dir_all(&root).expect("temp dir should be removed");
    }
}
