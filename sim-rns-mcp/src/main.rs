use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use sim_rns_control::*;
use std::path::PathBuf;

#[derive(Clone)]
struct Server {
    socket: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl Server {
    async fn forward(&self, request: Request) -> CallToolResult {
        let socket = self.socket.clone();
        match tokio::task::spawn_blocking(move || call(&socket, &request)).await {
            Ok(Ok(value)) => {
                if let (Some(image), Some(mime)) = (
                    value.get("image").and_then(|v| v.as_str()),
                    value.get("mime_type").and_then(|v| v.as_str()),
                ) {
                    CallToolResult::success(vec![ContentBlock::image(image, mime)])
                } else {
                    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
                }
            }
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(error)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
    }
}

#[tool_router]
impl Server {
    #[tool(
        description = "Start the local Sim RNS GUI, or attach if it is already running. Launches sim-rns-app next to this binary, inheriting the desktop display environment. Waits until its control endpoint is ready."
    )]
    async fn launch(&self) -> CallToolResult {
        let socket = self.socket.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            if call(&socket, &Request::App(AppRequest::State)).is_ok() {
                return Ok(());
            }
            let executable = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .with_file_name("sim-rns-app");
            let mut child = std::process::Command::new(executable)
                .env("SIM_RNS_CONTROL_SOCKET", &socket)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .map_err(|e| {
                    format!("failed to launch app: {e}; build sim-rns-app alongside sim-rns-mcp")
                })?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            for _ in 0..100 {
                if call(&socket, &Request::App(AppRequest::State)).is_ok() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(
                "app did not become ready; inspect stderr and the desktop display environment"
                    .into(),
            )
        })
        .await;
        match result {
            Ok(Ok(())) => self.forward(Request::App(AppRequest::State)).await,
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(error)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
    }
    #[tool(
        description = "Inspect the live Sim RNS app and active project, or quit the app. Requires the app to be running on the same machine."
    )]
    async fn app(&self, Parameters(request): Parameters<AppRequest>) -> CallToolResult {
        self.forward(Request::App(request)).await
    }
    #[tool(
        description = "Create/open/close/inspect projects, add node/script includes, and read/write project source files. Mutations require an idle app; write_file requires a stopped VM and rolls back invalid recipes. Paths for file operations are relative to the active project."
    )]
    async fn project(&self, Parameters(request): Parameters<ProjectRequest>) -> CallToolResult {
        self.forward(Request::Project(request)).await
    }
    #[tool(
        description = "Control the active project's VM and nodes; status includes topology, events and logs. Boot prepares storage automatically. Calls wait for completion. Snapshots and live topology changes are exposed but currently unsupported by the QEMU backend and return errors."
    )]
    async fn runtime(&self, Parameters(request): Parameters<RuntimeRequest>) -> CallToolResult {
        self.forward(Request::Runtime(request)).await
    }
    #[tool(
        description = "Inspect and control the GTK UI: discover widget/window IDs and commands with tree, then click buttons/tabs, edit text, toggle controls, select items, scroll, focus, invoke enabled commands or manage windows. Refresh tree after navigation. Operates in-app widgets; external OS file chooser dialogs are outside this API (use project.open/create). UI commands return after dispatch; runtime tools wait for completion."
    )]
    async fn ui(&self, Parameters(request): Parameters<UiRequest>) -> CallToolResult {
        self.forward(Request::Ui(request)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("sim-rns", env!("CARGO_PKG_VERSION")))
            .with_instructions("Control the running Sim RNS app. Start with app state and ui tree. All project/runtime operations target the active project. UI IDs are ephemeral. Runtime status contains logs. Do not retry timed-out mutations without inspecting state.")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = socket_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().ok_or("--socket requires a path")?.into(),
            "--help" | "-h" => {
                eprintln!("sim-rns-mcp [--socket PATH]\nMCP over stdio. Start sim-rns-app first. Default socket: {}", socket.display());
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    Server {
        socket,
        tool_router: Server::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
