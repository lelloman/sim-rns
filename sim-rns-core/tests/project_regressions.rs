use sim_rns_core::*;
use std::sync::{Arc, Barrier};

fn project(label: &str) -> Project {
    create_project(
        std::env::temp_dir().join(format!(
            "sim-rns-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
        label,
    )
    .unwrap()
}

#[test]
fn default_nodes_and_new_includes_use_supported_guest_templates() {
    let p = project("supported-defaults");
    let (p, _) = add_node_include(&p.root_path).unwrap();
    let recipe = project_recipe(&p).unwrap();
    assert_eq!(p.file.vm.ram_mb, 512);
    assert_eq!(p.file.vm.cpu_cores, 1);
    for element in recipe.elements {
        assert!([
            "network.lan",
            "reticulum.python.backbone",
            "script.python",
            "script.bash"
        ]
        .contains(&element.template_id.as_str()));
        assert!(element
            .assets
            .iter()
            .all(|asset| asset.mode == AssetMode::Copy));
    }
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn invalid_bundles_are_rejected_before_project_creation() {
    let p = project("bundle-validation");
    let bundle = p.root_path.join("bundle");
    std::fs::create_dir(&bundle).unwrap();
    let target = p.root_path.join("new-project");
    let manifest =
        serde_json::json!({"version":1,"kernel":"kernel","initrd":"initrd","disk":"disk"});
    for bytes in [
        b"not json".to_vec(),
        serde_json::to_vec(&serde_json::json!({"version":2})).unwrap(),
        serde_json::to_vec(&manifest).unwrap(),
    ] {
        std::fs::write(bundle.join("guest.json"), bytes).unwrap();
        assert!(create_project_with_bundle(&target, "Test", &bundle).is_err());
        assert!(!target.exists());
    }
    std::fs::write(bundle.join("kernel"), "kernel").unwrap();
    std::fs::write(bundle.join("initrd"), "").unwrap();
    assert!(validate_guest_bundle(&bundle)
        .unwrap_err()
        .contains("empty"));
    std::fs::remove_file(bundle.join("kernel")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", bundle.join("kernel")).unwrap();
    assert!(validate_guest_bundle(&bundle)
        .unwrap_err()
        .contains("inside"));
    assert!(!target.exists());
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn source_edits_validate_rollback_and_confine_paths() {
    let p = project("source-edit");
    let original = read_project_source(&p.root_path, PROJECT_FILE_NAME).unwrap();
    assert!(
        write_project_source(&p.root_path, PROJECT_FILE_NAME, "invalid")
            .unwrap_err()
            .contains("restored")
    );
    assert_eq!(
        read_project_source(&p.root_path, PROJECT_FILE_NAME).unwrap(),
        original
    );
    let mut changed = p.file.clone();
    changed.name = "Edited".into();
    assert_eq!(
        write_project_source(
            &p.root_path,
            PROJECT_FILE_NAME,
            &serde_json::to_string(&changed).unwrap()
        )
        .unwrap()
        .file
        .name,
        "Edited"
    );
    for path in ["../outside", "/etc/passwd", ".sim-rns/runtime-state.json"] {
        assert!(read_project_source(&p.root_path, path).is_err());
        assert!(write_project_source(&p.root_path, path, "no").is_err());
    }
    std::os::unix::fs::symlink("/etc/passwd", p.root_path.join("escape.txt")).unwrap();
    assert!(read_project_source(&p.root_path, "escape.txt").is_err());
    std::os::unix::fs::symlink(
        ".sim-rns/runtime-state.json",
        p.root_path.join("private.txt"),
    )
    .unwrap();
    assert!(read_project_source(&p.root_path, "private.txt").is_err());
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn parallel_commands_are_serialized_without_lost_updates() {
    let p = project("parallel");
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let p = p.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..25 {
                    FileBackedRuntime.execute(&p, RuntimeCommand::Boot).unwrap();
                    FileBackedRuntime.status(&p).unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let status = FileBackedRuntime.status(&p).unwrap();
    assert_eq!(status.recent_events[0].id, 200);
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn includes_preserve_implicit_startup() {
    let mut p = project("startup");
    p.file.startup.order.clear();
    persistence::atomic_write(
        &project_file_path(&p.root_path),
        &serde_json::to_vec(&p.file).unwrap(),
    )
    .unwrap();
    add_script_include(&p.root_path).unwrap();
    let (p, _) = add_node_include(&p.root_path).unwrap();
    assert!(p.file.startup.order.is_empty());
    assert_eq!(project_recipe(&p).unwrap().startup.order.len(), 6);
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn invalid_recipes_are_rejected() {
    let p = project("validation");
    let recipe = project_recipe(&p).unwrap();
    let mut broken = recipe.clone();
    broken.elements.push(broken.elements[0].clone());
    assert!(validate_recipe(&broken).is_err());
    let mut broken = recipe.clone();
    broken.elements[0].template_id = "unknown".into();
    assert!(validate_recipe(&broken).is_err());
    let mut broken = recipe.clone();
    broken.topology.attachments[0].network_id = "phone-a".into();
    assert!(validate_recipe(&broken).is_err());
    let mut broken = recipe.clone();
    broken.startup.order.push("missing".into());
    assert!(validate_recipe(&broken).is_err());
    let mut broken = recipe;
    broken.vm.cpu_cores = 0;
    assert!(validate_recipe(&broken).is_err());
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn missing_assets_and_symlink_escapes_are_rejected() {
    let mut p = project("assets");
    p.file.includes.assets.push("missing".into());
    assert!(project_recipe(&p).is_err());
    p.file.includes.assets.clear();
    std::os::unix::fs::symlink("/etc/passwd", p.root_path.join("assets/outside")).unwrap();
    p.file.includes.assets.push("assets/outside".into());
    assert!(project_recipe(&p).is_err());
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn editor_validates_conflicts_and_removal_without_losing_sources() {
    use editor::{apply, load, Edit};
    let p = project("node-editor");
    let snapshot = load(&p.root_path).unwrap();
    let mut node = snapshot.nodes[1].1.clone();
    node.id = "new-node".into();
    let save = |node| Edit::SaveNode {
        original_id: None,
        node,
    };
    let mut invalid = node.clone();
    invalid.attachments = vec!["missing-lan".into()];
    assert!(apply(&snapshot, save(invalid), false).is_err());
    assert_eq!(snapshot, load(&p.root_path).unwrap());
    let mut invalid = node.clone();
    invalid.id = "../escape".into();
    assert!(apply(&snapshot, save(invalid), false).is_err());
    apply(&snapshot, save(node.clone()), false).unwrap();
    assert!(apply(&snapshot, save(node.clone()), false)
        .unwrap_err()
        .contains("changed"));
    let next = load(&p.root_path).unwrap();
    assert!(apply(&next, save(node), false).is_err()); // duplicate ID
    let lan_path = next
        .nodes
        .iter()
        .find(|(_, n)| n.id == "lan-main")
        .unwrap()
        .0
        .clone();
    apply(
        &next,
        Edit::RemoveNode {
            id: "lan-main".into(),
        },
        false,
    )
    .unwrap();
    let next = load(&p.root_path).unwrap();
    assert!(next.nodes.iter().all(|(_, n)| n.attachments.is_empty()));
    assert!(!next.project.file.startup.order.contains(&"lan-main".into()));
    assert!(p.root_path.join(lan_path).exists());
    std::fs::remove_dir_all(p.root_path).unwrap();
}

#[test]
fn editor_archives_prepared_guest_only_with_explicit_choice() {
    use editor::{apply, load, Edit};
    let p = project("editor-archive");
    let layout = QemuRuntime::default().layout(&p);
    std::fs::create_dir_all(&layout.vm_dir).unwrap();
    std::fs::write(&layout.disk_image_path, "previous guest data").unwrap();
    let snapshot = load(&p.root_path).unwrap();
    let edit = Edit::SaveScript {
        path: snapshot.scripts[0].0.clone(),
        contents: "print('edited')\n".into(),
    };
    assert!(apply(&snapshot, edit.clone(), false)
        .unwrap_err()
        .contains("fresh guest"));
    assert_eq!(snapshot, load(&p.root_path).unwrap());
    apply(&snapshot, edit, true).unwrap();
    assert!(!layout.disk_image_path.exists());
    let backup = std::fs::read_dir(&layout.runtime_dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with("previous-vm-"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(
            backup
                .path()
                .join(layout.disk_image_path.file_name().unwrap())
        )
        .unwrap(),
        "previous guest data"
    );
    assert_eq!(
        load(&p.root_path).unwrap().scripts[0].1,
        "print('edited')\n"
    );
    std::fs::remove_dir_all(p.root_path).unwrap();
}
