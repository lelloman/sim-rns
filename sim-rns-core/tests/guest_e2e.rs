//! Run explicitly with SIM_RNS_TEST_BUNDLE=/path/to/bundle cargo test -p sim-rns-core --test guest_e2e -- --ignored --nocapture.
use sim_rns_core::*;
use std::time::{Duration, Instant};

struct RunningProject(Project);
impl Drop for RunningProject {
    fn drop(&mut self) {
        if let Err(error) = QemuRuntime::default().execute(&self.0, RuntimeCommand::Shutdown) {
            eprintln!(
                "guest test cleanup failed: {error}; project: {}",
                self.0.root_path.display()
            );
        }
    }
}

fn wait_for(project: &Project, check: impl Fn(&RuntimeStatus) -> bool) -> RuntimeStatus {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = QemuRuntime::default().status(project).unwrap();
        if check(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "guest condition timed out: {status:#?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "requires a built Linux guest bundle and QEMU"]
fn reticulum_guest_flow() {
    let bundle = std::env::var("SIM_RNS_TEST_BUNDLE").expect("set SIM_RNS_TEST_BUNDLE");
    let root = std::env::temp_dir().join(format!("sr-e2e-{}", std::process::id()));
    let mut project = create_project(&root, "Guest E2E").unwrap();
    project.file.vm.base_image = bundle;
    project.file.vm.ram_mb = 512;
    project.file.vm.cpu_cores = 1;
    for filename in ["backbone-a.node.json", "phone-a.node.json"] {
        let path = root.join("nodes").join(filename);
        let mut node: ProjectNodeFile =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        node.template_id = "reticulum.python.backbone".into();
        for asset in &mut node.assets {
            asset.mode = AssetMode::Copy;
        }
        node.resources = Some(ResourceLimits {
            memory_mb: 256,
            cpu_weight: 100,
        });
        persistence::atomic_write(&path, &serde_json::to_vec(&node).unwrap()).unwrap();
    }
    persistence::atomic_write(
        &project_file_path(&root),
        &serde_json::to_vec(&project.file).unwrap(),
    )
    .unwrap();
    let runtime = QemuRuntime::default();
    runtime
        .execute(
            &project,
            RuntimeCommand::PrepareVm {
                source_image: None,
                size_gb: 0,
            },
        )
        .unwrap();
    let guard = RunningProject(project.clone());
    runtime.execute(&project, RuntimeCommand::Boot).unwrap();
    let received = wait_for(&project, |status| {
        [("backbone-a", "phone-a"), ("phone-a", "backbone-a")]
            .iter()
            .all(|(node, peer)| {
                status.node_logs.get(*node).is_some_and(|lines| {
                    lines
                        .iter()
                        .any(|line| line.starts_with(&format!("RECEIVED hello from {peer} at ")))
                })
            })
    });
    assert_eq!(received.backend_state, RuntimeBackendState::Reachable);
    println!("Both Reticulum nodes exchanged real packets.");
    let identity = |status: &RuntimeStatus, id: &str| {
        status
            .node_logs
            .get(id)?
            .windows(2)
            .rev()
            .find(|pair| pair[0].starts_with("BOOT ") && pair[1].starts_with("IDENTITY "))
            .map(|pair| (pair[0].clone(), pair[1].clone()))
    };
    let first_a = identity(&received, "backbone-a").expect("first identity logged");
    let first_b = identity(&received, "phone-a").expect("second identity logged");
    assert_ne!(first_a.1, first_b.1);
    let isolation = |id: &str| {
        received.node_logs[id]
            .iter()
            .find(|line| line.starts_with("ISOLATION "))
            .unwrap()
            .clone()
    };
    let isolation_a = isolation("backbone-a");
    let isolation_b = isolation("phone-a");
    assert!(isolation_a.contains("uid=1001 "));
    assert!(isolation_b.contains("uid=1002 "));
    assert_ne!(
        isolation_a.split("netns=").nth(1),
        isolation_b.split("netns=").nth(1)
    );
    runtime
        .execute(
            &project,
            RuntimeCommand::StopNode {
                element_id: "phone-a".into(),
            },
        )
        .unwrap();
    let stopped = runtime.status(&project).unwrap();
    assert_eq!(
        stopped
            .nodes
            .iter()
            .find(|n| n.element_id == "phone-a")
            .unwrap()
            .state,
        NodeRuntimeState::Stopped
    );
    assert_eq!(
        stopped
            .nodes
            .iter()
            .find(|n| n.element_id == "backbone-a")
            .unwrap()
            .state,
        NodeRuntimeState::Running
    );
    runtime
        .execute(
            &project,
            RuntimeCommand::StartNode {
                element_id: "phone-a".into(),
            },
        )
        .unwrap();
    assert_eq!(
        runtime
            .execute(&project, RuntimeCommand::Pause)
            .unwrap()
            .status
            .vm_state,
        RuntimeVmState::Paused
    );
    assert_eq!(
        runtime
            .execute(&project, RuntimeCommand::Resume)
            .unwrap()
            .status
            .vm_state,
        RuntimeVmState::Running
    );
    runtime.execute(&project, RuntimeCommand::Shutdown).unwrap();
    runtime.execute(&project, RuntimeCommand::Boot).unwrap();
    let restarted = wait_for(&project, |status| {
        identity(status, "backbone-a")
            .is_some_and(|(boot, key)| boot != first_a.0 && key == first_a.1)
            && identity(status, "phone-a")
                .is_some_and(|(boot, key)| boot != first_b.0 && key == first_b.1)
    });
    assert_eq!(restarted.backend_state, RuntimeBackendState::Reachable);
    println!("Node lifecycle, pause/resume, and identities across VM reboot verified.");
    drop(guard);
    assert_eq!(
        runtime.status(&project).unwrap().vm_state,
        RuntimeVmState::Stopped
    );
    std::fs::remove_dir_all(root).unwrap();
}
