use sim_rns_core::*;

fn main() {
    if let Err(error) = run() {
        eprintln!("sim-rns: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err("usage: sim-rns-ctl <create-demo|status|boot|pause|resume|stop|start-node|stop-node|restart-node> PROJECT [IMAGE_BUNDLE|ELEMENT_ID]".into());
    }
    if args[0] == "create-demo" {
        let image = args
            .get(2)
            .ok_or("create-demo requires a guest image bundle")?;
        let project = create_demo_project(&args[1], image)?;
        println!("{}", project.root_path.display());
        return Ok(());
    }
    let project = load_project(&args[1])?;
    let runtime = QemuRuntime::default();
    if args[0] == "status" {
        println!(
            "{}",
            serde_json::to_string_pretty(&runtime.status(&project).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let element_id = || {
        args.get(2)
            .cloned()
            .ok_or_else(|| "node command requires ELEMENT_ID".to_string())
    };
    let command = match args[0].as_str() {
        "boot" => {
            if !runtime
                .status(&project)
                .map_err(|e| e.to_string())?
                .vm_assets
                .prepared
            {
                runtime
                    .execute(
                        &project,
                        RuntimeCommand::PrepareVm {
                            source_image: None,
                            size_gb: 0,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            RuntimeCommand::Boot
        }
        "pause" => RuntimeCommand::Pause,
        "resume" => RuntimeCommand::Resume,
        "stop" => RuntimeCommand::Shutdown,
        "start-node" => RuntimeCommand::StartNode {
            element_id: element_id()?,
        },
        "stop-node" => RuntimeCommand::StopNode {
            element_id: element_id()?,
        },
        "restart-node" => RuntimeCommand::RestartNode {
            element_id: element_id()?,
        },
        _ => return Err("unknown command".into()),
    };
    let outcome = runtime
        .execute(&project, command)
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&outcome).map_err(|e| e.to_string())?
    );
    Ok(())
}
