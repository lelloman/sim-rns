use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{project_recipe, Attachment, Project, Recipe};

const RUNTIME_SCHEMA_VERSION: u32 = 1;
const RUNTIME_DIR: &str = ".sim-rns";
const RUNTIME_STATE_FILE: &str = "runtime-state.json";
const VM_DIR: &str = "vm";
const SNAPSHOTS_DIR: &str = "snapshots";
const LOGS_DIR: &str = "logs";
const VM_DISK_FILE: &str = "disk.qcow2";
const VM_PID_FILE: &str = "qemu.pid";
const VM_QMP_SOCKET_FILE: &str = "qmp.sock";
const VM_LOG_FILE: &str = "qemu.log";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeVmState {
    Stopped,
    Running,
    Paused,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackendState {
    Offline,
    Reachable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeRuntimeState {
    Disabled,
    Stopped,
    Running,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeStatus {
    pub element_id: String,
    pub template_id: String,
    pub enabled: bool,
    pub state: NodeRuntimeState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTopologyOverlay {
    #[serde(default)]
    pub additions: Vec<Attachment>,
    #[serde(default)]
    pub removals: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub id: String,
    pub name: String,
    pub note: Option<String>,
    pub created_at_unix_ms: u64,
    pub vm_state: RuntimeVmState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEvent {
    pub id: u64,
    pub timestamp_unix_ms: u64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub project_id: String,
    pub project_name: String,
    pub vm_state: RuntimeVmState,
    pub backend_state: RuntimeBackendState,
    pub vm_assets: RuntimeVmAssets,
    pub nodes: Vec<NodeRuntimeStatus>,
    pub effective_topology: Vec<Attachment>,
    pub topology_overlay: RuntimeTopologyOverlay,
    pub snapshots: Vec<RuntimeSnapshot>,
    pub recent_events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeVmAssets {
    pub prepared: bool,
    pub disk_image_path: String,
    pub pid_path: String,
    pub qmp_socket_path: String,
    pub log_path: String,
    pub snapshots_dir: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeCommand {
    PrepareVm {
        source_image: Option<String>,
        size_gb: u32,
    },
    Boot,
    Shutdown,
    Pause,
    Resume,
    CreateSnapshot {
        name: String,
        note: Option<String>,
    },
    RestoreSnapshot {
        snapshot_id: String,
    },
    DeleteSnapshot {
        snapshot_id: String,
    },
    StartNode {
        element_id: String,
    },
    StopNode {
        element_id: String,
    },
    RestartNode {
        element_id: String,
    },
    AddTopologyLink {
        element_id: String,
        network_id: String,
    },
    RemoveTopologyLink {
        element_id: String,
        network_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCommandOutcome {
    pub command_id: u64,
    pub timestamp_unix_ms: u64,
    pub accepted: bool,
    pub message: Option<String>,
    pub status: RuntimeStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    Validation(String),
    Unavailable(String),
    Unsupported(String),
    Persistence(String),
    ProjectLoad(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message)
            | Self::Unavailable(message)
            | Self::Unsupported(message)
            | Self::Persistence(message)
            | Self::ProjectLoad(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub trait ProjectRuntime {
    fn status(&self, project: &Project) -> Result<RuntimeStatus, RuntimeError>;

    fn execute(
        &self,
        project: &Project,
        command: RuntimeCommand,
    ) -> Result<RuntimeCommandOutcome, RuntimeError>;
}

#[derive(Clone, Debug, Default)]
pub struct FileBackedRuntime;

#[derive(Clone, Debug)]
pub struct QemuRuntime {
    qemu_binary: String,
    qemu_img_binary: String,
}

impl Default for QemuRuntime {
    fn default() -> Self {
        Self {
            qemu_binary: "qemu-system-x86_64".to_string(),
            qemu_img_binary: "qemu-img".to_string(),
        }
    }
}

impl QemuRuntime {
    pub fn new(qemu_binary: impl Into<String>) -> Self {
        Self {
            qemu_binary: qemu_binary.into(),
            qemu_img_binary: "qemu-img".to_string(),
        }
    }

    pub fn with_qemu_img_binary(mut self, qemu_img_binary: impl Into<String>) -> Self {
        self.qemu_img_binary = qemu_img_binary.into();
        self
    }

    pub fn layout(&self, project: &Project) -> RuntimeVmLayout {
        RuntimeVmLayout::for_project(project)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeVmLayout {
    pub runtime_dir: PathBuf,
    pub vm_dir: PathBuf,
    pub snapshots_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub disk_image_path: PathBuf,
    pub pid_path: PathBuf,
    pub qmp_socket_path: PathBuf,
    pub log_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeState {
    schema_version: u32,
    project_id: String,
    project_name: String,
    vm_state: RuntimeVmState,
    backend_state: RuntimeBackendState,
    nodes: Vec<NodeRuntimeStatus>,
    topology_overlay: RuntimeTopologyOverlay,
    snapshots: Vec<RuntimeSnapshot>,
    events: Vec<RuntimeEvent>,
    command_clock: u64,
}

impl ProjectRuntime for FileBackedRuntime {
    fn status(&self, project: &Project) -> Result<RuntimeStatus, RuntimeError> {
        let recipe = project_recipe(project).map_err(RuntimeError::ProjectLoad)?;
        let state = load_or_init_state(project, &recipe)?;
        Ok(status_from_state(project, &state, &recipe))
    }

    fn execute(
        &self,
        project: &Project,
        command: RuntimeCommand,
    ) -> Result<RuntimeCommandOutcome, RuntimeError> {
        let recipe = project_recipe(project).map_err(RuntimeError::ProjectLoad)?;
        let mut state = load_or_init_state(project, &recipe)?;
        let timestamp = unix_time_ms()?;
        let command_id = next_command_id(&mut state);
        let message = apply_command(&mut state, &recipe, command, timestamp)?;
        push_event(&mut state, command_id, timestamp, message.clone());
        save_state(project, &state)?;
        Ok(RuntimeCommandOutcome {
            command_id,
            timestamp_unix_ms: timestamp,
            accepted: true,
            message: Some(message),
            status: status_from_state(project, &state, &recipe),
        })
    }
}

impl ProjectRuntime for QemuRuntime {
    fn status(&self, project: &Project) -> Result<RuntimeStatus, RuntimeError> {
        let mut status = FileBackedRuntime.status(project)?;
        let layout = self.layout(project);
        let process_running = qemu_process_is_running(&layout)?;
        status.vm_assets = vm_assets(&layout);
        if process_running {
            if status.vm_state == RuntimeVmState::Stopped {
                status.vm_state = RuntimeVmState::Running;
                status.backend_state = RuntimeBackendState::Reachable;
            }
        } else {
            status.vm_state = RuntimeVmState::Stopped;
            status.backend_state = RuntimeBackendState::Offline;
            for node in &mut status.nodes {
                if node.enabled {
                    node.state = NodeRuntimeState::Stopped;
                }
            }
        }
        Ok(status)
    }

    fn execute(
        &self,
        project: &Project,
        command: RuntimeCommand,
    ) -> Result<RuntimeCommandOutcome, RuntimeError> {
        match command {
            RuntimeCommand::PrepareVm {
                source_image,
                size_gb,
            } => {
                self.prepare_vm(project, source_image, size_gb)?;
                FileBackedRuntime.execute(
                    project,
                    RuntimeCommand::PrepareVm {
                        source_image: None,
                        size_gb,
                    },
                )
            }
            RuntimeCommand::Boot => {
                self.boot(project)?;
                FileBackedRuntime.execute(project, RuntimeCommand::Boot)
            }
            RuntimeCommand::Shutdown => {
                self.shutdown(project)?;
                FileBackedRuntime.execute(project, RuntimeCommand::Shutdown)
            }
            RuntimeCommand::Pause => {
                self.qmp_execute(project, "stop")?;
                FileBackedRuntime.execute(project, RuntimeCommand::Pause)
            }
            RuntimeCommand::Resume => {
                self.qmp_execute(project, "cont")?;
                FileBackedRuntime.execute(project, RuntimeCommand::Resume)
            }
            command => FileBackedRuntime.execute(project, command),
        }
    }
}

impl QemuRuntime {
    fn prepare_vm(
        &self,
        project: &Project,
        source_image: Option<String>,
        size_gb: u32,
    ) -> Result<(), RuntimeError> {
        let layout = self.layout(project);
        ensure_runtime_layout(&layout)?;
        if layout.disk_image_path.exists() {
            return Err(RuntimeError::Validation(format!(
                "VM disk image already exists at {}",
                layout.disk_image_path.display()
            )));
        }
        if let Some(source_image) = source_image {
            let source_path = PathBuf::from(source_image);
            if !source_path.is_file() {
                return Err(RuntimeError::Validation(format!(
                    "source VM image does not exist at {}",
                    source_path.display()
                )));
            }
            std::fs::copy(&source_path, &layout.disk_image_path).map_err(|error| {
                RuntimeError::Persistence(format!(
                    "failed to import {} to {}: {error}",
                    source_path.display(),
                    layout.disk_image_path.display()
                ))
            })?;
            return Ok(());
        }

        let disk_size_gb = if size_gb == 0 { 8 } else { size_gb };
        let status = Command::new(&self.qemu_img_binary)
            .arg("create")
            .arg("-f")
            .arg("qcow2")
            .arg(&layout.disk_image_path)
            .arg(format!("{disk_size_gb}G"))
            .status()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "failed to run `{}`: {error}",
                    self.qemu_img_binary
                ))
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(RuntimeError::Unavailable(format!(
                "`{}` failed to create {}",
                self.qemu_img_binary,
                layout.disk_image_path.display()
            )))
        }
    }

    fn boot(&self, project: &Project) -> Result<(), RuntimeError> {
        let layout = self.layout(project);
        ensure_runtime_layout(&layout)?;
        if qemu_process_is_running(&layout)? {
            return Err(RuntimeError::Unavailable(
                "project VM is already running".to_string(),
            ));
        }
        if !layout.disk_image_path.is_file() {
            return Err(RuntimeError::Validation(format!(
                "VM disk image is missing at {}; create or import it before booting",
                layout.disk_image_path.display()
            )));
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&layout.log_path)
            .map_err(|error| {
                RuntimeError::Persistence(format!(
                    "failed to open {}: {error}",
                    layout.log_path.display()
                ))
            })?;
        let log_for_stderr = log.try_clone().map_err(|error| {
            RuntimeError::Persistence(format!(
                "failed to clone {}: {error}",
                layout.log_path.display()
            ))
        })?;
        let _ = std::fs::remove_file(&layout.qmp_socket_path);
        let child = Command::new(&self.qemu_binary)
            .arg("-name")
            .arg(format!("sim-rns-{}", project.file.project_id))
            .arg("-m")
            .arg(project.file.vm.ram_mb.to_string())
            .arg("-smp")
            .arg(project.file.vm.cpu_cores.to_string())
            .arg("-drive")
            .arg(format!(
                "file={},if=virtio,format=qcow2",
                layout.disk_image_path.display()
            ))
            .arg("-qmp")
            .arg(format!(
                "unix:{},server,nowait",
                layout.qmp_socket_path.display()
            ))
            .arg("-display")
            .arg("none")
            .arg("-serial")
            .arg("mon:stdio")
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_for_stderr))
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "failed to start `{}`: {error}",
                    self.qemu_binary
                ))
            })?;
        std::fs::write(&layout.pid_path, child.id().to_string()).map_err(|error| {
            RuntimeError::Persistence(format!(
                "failed to write {}: {error}",
                layout.pid_path.display()
            ))
        })?;
        Ok(())
    }

    fn shutdown(&self, project: &Project) -> Result<(), RuntimeError> {
        let layout = self.layout(project);
        if !qemu_process_is_running(&layout)? {
            cleanup_stale_vm_files(&layout);
            return Ok(());
        }
        if self.qmp_execute(project, "quit").is_err() {
            let pid = read_pid(&layout)?;
            Command::new("kill")
                .arg(pid.to_string())
                .status()
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("failed to run kill: {error}"))
                })?;
        } else if !wait_for_qemu_exit(&layout)? {
            let pid = read_pid(&layout)?;
            Command::new("kill")
                .arg(pid.to_string())
                .status()
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("failed to run kill: {error}"))
                })?;
        }
        cleanup_stale_vm_files(&layout);
        Ok(())
    }

    fn qmp_execute(&self, project: &Project, command: &str) -> Result<(), RuntimeError> {
        let layout = self.layout(project);
        if !qemu_process_is_running(&layout)? {
            return Err(RuntimeError::Unavailable(
                "project VM is not running".to_string(),
            ));
        }
        let mut stream = UnixStream::connect(&layout.qmp_socket_path).map_err(|error| {
            RuntimeError::Unavailable(format!(
                "failed to connect to QMP socket {}: {error}",
                layout.qmp_socket_path.display()
            ))
        })?;
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| {
                RuntimeError::Unavailable(format!("failed to set QMP timeout: {error}"))
            })?;
        let mut greeting = [0_u8; 4096];
        let _ = stream.read(&mut greeting);
        stream
            .write_all(b"{\"execute\":\"qmp_capabilities\"}\n")
            .map_err(|error| {
                RuntimeError::Unavailable(format!("failed to send QMP capabilities: {error}"))
            })?;
        let _ = stream.read(&mut greeting);
        let payload = format!("{{\"execute\":\"{command}\"}}\n");
        stream.write_all(payload.as_bytes()).map_err(|error| {
            RuntimeError::Unavailable(format!("failed to send QMP command `{command}`: {error}"))
        })?;
        Ok(())
    }
}

fn load_or_init_state(project: &Project, recipe: &Recipe) -> Result<RuntimeState, RuntimeError> {
    let path = runtime_state_path(project);
    let state = if path.is_file() {
        let payload = std::fs::read_to_string(&path).map_err(|error| {
            RuntimeError::Persistence(format!("failed to read {}: {error}", path.display()))
        })?;
        let state = serde_json::from_str::<RuntimeState>(&payload).map_err(|error| {
            RuntimeError::Persistence(format!("failed to parse {}: {error}", path.display()))
        })?;
        if state.schema_version != RUNTIME_SCHEMA_VERSION {
            return Err(RuntimeError::Persistence(format!(
                "unsupported runtime schema version {} in {}",
                state.schema_version,
                path.display()
            )));
        }
        state
    } else {
        initial_state(project, recipe)
    };
    let mut state = state;
    reconcile_state(&mut state, project, recipe);
    save_state(project, &state)?;
    Ok(state)
}

fn initial_state(project: &Project, recipe: &Recipe) -> RuntimeState {
    RuntimeState {
        schema_version: RUNTIME_SCHEMA_VERSION,
        project_id: project.file.project_id.clone(),
        project_name: project.file.name.clone(),
        vm_state: RuntimeVmState::Stopped,
        backend_state: RuntimeBackendState::Offline,
        nodes: recipe
            .elements
            .iter()
            .map(|element| NodeRuntimeStatus {
                element_id: element.id.clone(),
                template_id: element.template_id.clone(),
                enabled: element.enabled,
                state: if element.enabled {
                    NodeRuntimeState::Stopped
                } else {
                    NodeRuntimeState::Disabled
                },
            })
            .collect(),
        topology_overlay: RuntimeTopologyOverlay::default(),
        snapshots: Vec::new(),
        events: Vec::new(),
        command_clock: 0,
    }
}

fn reconcile_state(state: &mut RuntimeState, project: &Project, recipe: &Recipe) {
    state.project_id = project.file.project_id.clone();
    state.project_name = project.file.name.clone();

    let elements = recipe
        .elements
        .iter()
        .map(|element| {
            (
                element.id.clone(),
                (element.template_id.clone(), element.enabled),
            )
        })
        .collect::<BTreeMap<_, _>>();

    state
        .nodes
        .retain(|node| elements.contains_key(&node.element_id));

    for node in &mut state.nodes {
        if let Some((template_id, enabled)) = elements.get(&node.element_id) {
            node.template_id = template_id.clone();
            node.enabled = *enabled;
            if !node.enabled {
                node.state = NodeRuntimeState::Disabled;
            } else if node.state == NodeRuntimeState::Disabled {
                node.state = NodeRuntimeState::Stopped;
            }
        }
    }

    for (element_id, (template_id, enabled)) in elements {
        if state.nodes.iter().any(|node| node.element_id == element_id) {
            continue;
        }
        state.nodes.push(NodeRuntimeStatus {
            element_id,
            template_id,
            enabled,
            state: if enabled {
                NodeRuntimeState::Stopped
            } else {
                NodeRuntimeState::Disabled
            },
        });
    }
}

fn apply_command(
    state: &mut RuntimeState,
    recipe: &Recipe,
    command: RuntimeCommand,
    timestamp: u64,
) -> Result<String, RuntimeError> {
    match command {
        RuntimeCommand::PrepareVm { .. } => Ok("VM assets prepared.".to_string()),
        RuntimeCommand::Boot => {
            state.vm_state = RuntimeVmState::Running;
            state.backend_state = RuntimeBackendState::Reachable;
            let startup = recipe
                .startup
                .order
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            for node in &mut state.nodes {
                if node.enabled && startup.contains(&node.element_id) {
                    node.state = NodeRuntimeState::Running;
                }
            }
            Ok("Project booted.".to_string())
        }
        RuntimeCommand::Shutdown => {
            state.vm_state = RuntimeVmState::Stopped;
            state.backend_state = RuntimeBackendState::Offline;
            for node in &mut state.nodes {
                if node.enabled {
                    node.state = NodeRuntimeState::Stopped;
                }
            }
            Ok("Project shut down.".to_string())
        }
        RuntimeCommand::Pause => {
            require_vm_state(state, RuntimeVmState::Running, "pause")?;
            state.vm_state = RuntimeVmState::Paused;
            state.backend_state = RuntimeBackendState::Offline;
            Ok("Project paused.".to_string())
        }
        RuntimeCommand::Resume => {
            require_vm_state(state, RuntimeVmState::Paused, "resume")?;
            state.vm_state = RuntimeVmState::Running;
            state.backend_state = RuntimeBackendState::Reachable;
            Ok("Project resumed.".to_string())
        }
        RuntimeCommand::CreateSnapshot { name, note } => {
            let snapshot_name = name.trim();
            if snapshot_name.is_empty() {
                return Err(RuntimeError::Validation(
                    "snapshot name cannot be empty".to_string(),
                ));
            }
            let snapshot = RuntimeSnapshot {
                id: format!("snapshot-{timestamp}"),
                name: snapshot_name.to_string(),
                note,
                created_at_unix_ms: timestamp,
                vm_state: state.vm_state.clone(),
            };
            state.snapshots.insert(0, snapshot);
            Ok(format!("Snapshot `{snapshot_name}` created."))
        }
        RuntimeCommand::RestoreSnapshot { snapshot_id } => {
            let snapshot = state
                .snapshots
                .iter()
                .find(|snapshot| snapshot.id == snapshot_id)
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::Validation(format!("snapshot `{snapshot_id}` does not exist"))
                })?;
            state.vm_state = snapshot.vm_state;
            state.backend_state = if state.vm_state == RuntimeVmState::Running {
                RuntimeBackendState::Reachable
            } else {
                RuntimeBackendState::Offline
            };
            Ok(format!("Snapshot `{}` restored.", snapshot.name))
        }
        RuntimeCommand::DeleteSnapshot { snapshot_id } => {
            let before = state.snapshots.len();
            state
                .snapshots
                .retain(|snapshot| snapshot.id != snapshot_id);
            if before == state.snapshots.len() {
                return Err(RuntimeError::Validation(format!(
                    "snapshot `{snapshot_id}` does not exist"
                )));
            }
            Ok(format!("Snapshot `{snapshot_id}` deleted."))
        }
        RuntimeCommand::StartNode { element_id } => {
            require_vm_state(state, RuntimeVmState::Running, "start nodes")?;
            let node = find_enabled_node_mut(state, &element_id)?;
            node.state = NodeRuntimeState::Running;
            Ok(format!("Node `{element_id}` started."))
        }
        RuntimeCommand::StopNode { element_id } => {
            let node = find_enabled_node_mut(state, &element_id)?;
            node.state = NodeRuntimeState::Stopped;
            Ok(format!("Node `{element_id}` stopped."))
        }
        RuntimeCommand::RestartNode { element_id } => {
            require_vm_state(state, RuntimeVmState::Running, "restart nodes")?;
            let node = find_enabled_node_mut(state, &element_id)?;
            node.state = NodeRuntimeState::Running;
            Ok(format!("Node `{element_id}` restarted."))
        }
        RuntimeCommand::AddTopologyLink {
            element_id,
            network_id,
        } => {
            validate_element(recipe, &element_id)?;
            validate_element(recipe, &network_id)?;
            let link = Attachment {
                element_id,
                network_id,
            };
            state
                .topology_overlay
                .removals
                .retain(|entry| entry != &link);
            if !effective_topology(recipe, &state.topology_overlay).contains(&link)
                && !state.topology_overlay.additions.contains(&link)
            {
                state.topology_overlay.additions.push(link.clone());
            }
            Ok(format!(
                "Topology link `{} -> {}` added.",
                link.element_id, link.network_id
            ))
        }
        RuntimeCommand::RemoveTopologyLink {
            element_id,
            network_id,
        } => {
            validate_element(recipe, &element_id)?;
            validate_element(recipe, &network_id)?;
            let link = Attachment {
                element_id,
                network_id,
            };
            state
                .topology_overlay
                .additions
                .retain(|entry| entry != &link);
            if recipe.topology.attachments.contains(&link)
                && !state.topology_overlay.removals.contains(&link)
            {
                state.topology_overlay.removals.push(link.clone());
            }
            Ok(format!(
                "Topology link `{} -> {}` removed.",
                link.element_id, link.network_id
            ))
        }
    }
}

fn require_vm_state(
    state: &RuntimeState,
    expected: RuntimeVmState,
    action: &str,
) -> Result<(), RuntimeError> {
    if state.vm_state == expected {
        Ok(())
    } else {
        Err(RuntimeError::Unavailable(format!(
            "cannot {action} while VM is {:?}",
            state.vm_state
        )))
    }
}

fn find_enabled_node_mut<'a>(
    state: &'a mut RuntimeState,
    element_id: &str,
) -> Result<&'a mut NodeRuntimeStatus, RuntimeError> {
    let node = state
        .nodes
        .iter_mut()
        .find(|node| node.element_id == element_id)
        .ok_or_else(|| RuntimeError::Validation(format!("node `{element_id}` does not exist")))?;
    if !node.enabled {
        return Err(RuntimeError::Validation(format!(
            "node `{element_id}` is disabled"
        )));
    }
    Ok(node)
}

fn validate_element(recipe: &Recipe, element_id: &str) -> Result<(), RuntimeError> {
    if recipe
        .elements
        .iter()
        .any(|element| element.id == element_id)
    {
        Ok(())
    } else {
        Err(RuntimeError::Validation(format!(
            "element `{element_id}` does not exist"
        )))
    }
}

fn status_from_state(project: &Project, state: &RuntimeState, recipe: &Recipe) -> RuntimeStatus {
    let layout = RuntimeVmLayout::for_project(project);
    RuntimeStatus {
        project_id: state.project_id.clone(),
        project_name: state.project_name.clone(),
        vm_state: state.vm_state.clone(),
        backend_state: state.backend_state.clone(),
        vm_assets: vm_assets(&layout),
        nodes: state.nodes.clone(),
        effective_topology: effective_topology(recipe, &state.topology_overlay),
        topology_overlay: state.topology_overlay.clone(),
        snapshots: state.snapshots.clone(),
        recent_events: state.events.iter().rev().take(20).cloned().collect(),
    }
}

fn effective_topology(recipe: &Recipe, overlay: &RuntimeTopologyOverlay) -> Vec<Attachment> {
    let removals = overlay.removals.iter().collect::<BTreeSet<_>>();
    let mut links = recipe
        .topology
        .attachments
        .iter()
        .filter(|link| !removals.contains(link))
        .cloned()
        .collect::<Vec<_>>();
    for link in &overlay.additions {
        if !links.contains(link) {
            links.push(link.clone());
        }
    }
    links
}

fn next_command_id(state: &mut RuntimeState) -> u64 {
    state.command_clock = state.command_clock.saturating_add(1);
    state.command_clock
}

fn push_event(state: &mut RuntimeState, id: u64, timestamp: u64, message: String) {
    state.events.push(RuntimeEvent {
        id,
        timestamp_unix_ms: timestamp,
        message,
    });
    if state.events.len() > 100 {
        let overflow = state.events.len() - 100;
        state.events.drain(0..overflow);
    }
}

fn save_state(project: &Project, state: &RuntimeState) -> Result<(), RuntimeError> {
    let path = runtime_state_path(project);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            RuntimeError::Persistence(format!("failed to create {}: {error}", parent.display()))
        })?;
    }
    let payload = serde_json::to_string_pretty(state).map_err(|error| {
        RuntimeError::Persistence(format!("failed to serialize runtime state: {error}"))
    })?;
    std::fs::write(&path, payload).map_err(|error| {
        RuntimeError::Persistence(format!("failed to write {}: {error}", path.display()))
    })
}

fn runtime_state_path(project: &Project) -> PathBuf {
    project.root_path.join(RUNTIME_DIR).join(RUNTIME_STATE_FILE)
}

impl RuntimeVmLayout {
    fn for_project(project: &Project) -> Self {
        Self::for_project_path(&project.root_path)
    }

    fn for_project_path(root_path: &std::path::Path) -> Self {
        let runtime_dir = root_path.join(RUNTIME_DIR);
        let vm_dir = runtime_dir.join(VM_DIR);
        let snapshots_dir = runtime_dir.join(SNAPSHOTS_DIR);
        let logs_dir = runtime_dir.join(LOGS_DIR);
        Self {
            runtime_dir,
            disk_image_path: vm_dir.join(VM_DISK_FILE),
            pid_path: vm_dir.join(VM_PID_FILE),
            qmp_socket_path: vm_dir.join(VM_QMP_SOCKET_FILE),
            log_path: logs_dir.join(VM_LOG_FILE),
            vm_dir,
            snapshots_dir,
            logs_dir,
        }
    }
}

fn ensure_runtime_layout(layout: &RuntimeVmLayout) -> Result<(), RuntimeError> {
    for dir in [&layout.vm_dir, &layout.snapshots_dir, &layout.logs_dir] {
        std::fs::create_dir_all(dir).map_err(|error| {
            RuntimeError::Persistence(format!("failed to create {}: {error}", dir.display()))
        })?;
    }
    Ok(())
}

fn vm_assets(layout: &RuntimeVmLayout) -> RuntimeVmAssets {
    RuntimeVmAssets {
        prepared: layout.disk_image_path.is_file(),
        disk_image_path: layout.disk_image_path.to_string_lossy().into_owned(),
        pid_path: layout.pid_path.to_string_lossy().into_owned(),
        qmp_socket_path: layout.qmp_socket_path.to_string_lossy().into_owned(),
        log_path: layout.log_path.to_string_lossy().into_owned(),
        snapshots_dir: layout.snapshots_dir.to_string_lossy().into_owned(),
    }
}

fn read_pid(layout: &RuntimeVmLayout) -> Result<u32, RuntimeError> {
    let payload = std::fs::read_to_string(&layout.pid_path).map_err(|error| {
        RuntimeError::Persistence(format!(
            "failed to read {}: {error}",
            layout.pid_path.display()
        ))
    })?;
    payload.trim().parse::<u32>().map_err(|error| {
        RuntimeError::Persistence(format!(
            "failed to parse pid from {}: {error}",
            layout.pid_path.display()
        ))
    })
}

fn qemu_process_is_running(layout: &RuntimeVmLayout) -> Result<bool, RuntimeError> {
    if !layout.pid_path.is_file() {
        return Ok(false);
    }
    let pid = read_pid(layout)?;
    let status = Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map_err(|error| RuntimeError::Unavailable(format!("failed to run kill -0: {error}")))?;
    if status.success() {
        Ok(true)
    } else {
        cleanup_stale_vm_files(layout);
        Ok(false)
    }
}

fn cleanup_stale_vm_files(layout: &RuntimeVmLayout) {
    let _ = std::fs::remove_file(&layout.pid_path);
    let _ = std::fs::remove_file(&layout.qmp_socket_path);
}

fn wait_for_qemu_exit(layout: &RuntimeVmLayout) -> Result<bool, RuntimeError> {
    for _ in 0..20 {
        if !qemu_process_is_running(layout)? {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(false)
}

fn unix_time_ms() -> Result<u64, RuntimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| RuntimeError::Persistence(format!("system clock error: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{
        FileBackedRuntime, NodeRuntimeState, ProjectRuntime, QemuRuntime, RuntimeBackendState,
        RuntimeCommand, RuntimeError, RuntimeVmState,
    };
    use crate::{create_project, project_file_path, project_recipe};

    fn unique_test_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    #[test]
    fn runtime_initializes_from_project_recipe() {
        let root = unique_test_dir("sim-rns-runtime-init");
        let project = create_project(&root, "Runtime Init").expect("project should be created");
        let runtime = FileBackedRuntime;
        let status = runtime.status(&project).expect("status should load");
        let recipe = project_recipe(&project).expect("recipe should load");

        assert_eq!(status.project_id, project.file.project_id);
        assert_eq!(status.vm_state, RuntimeVmState::Stopped);
        assert_eq!(status.nodes.len(), recipe.elements.len());
        assert!(root.join(".sim-rns/runtime-state.json").is_file());
    }

    #[test]
    fn runtime_vm_lifecycle_updates_state() {
        let root = unique_test_dir("sim-rns-runtime-lifecycle");
        let project =
            create_project(&root, "Runtime Lifecycle").expect("project should be created");
        let runtime = FileBackedRuntime;

        let booted = runtime
            .execute(&project, RuntimeCommand::Boot)
            .expect("boot should succeed")
            .status;
        assert_eq!(booted.vm_state, RuntimeVmState::Running);
        assert!(
            booted
                .nodes
                .iter()
                .any(|node| node.element_id == "backbone-a"
                    && node.state == NodeRuntimeState::Running)
        );

        let paused = runtime
            .execute(&project, RuntimeCommand::Pause)
            .expect("pause should succeed")
            .status;
        assert_eq!(paused.vm_state, RuntimeVmState::Paused);

        let resumed = runtime
            .execute(&project, RuntimeCommand::Resume)
            .expect("resume should succeed")
            .status;
        assert_eq!(resumed.vm_state, RuntimeVmState::Running);

        let stopped = runtime
            .execute(&project, RuntimeCommand::Shutdown)
            .expect("shutdown should succeed")
            .status;
        assert_eq!(stopped.vm_state, RuntimeVmState::Stopped);
        assert!(stopped
            .nodes
            .iter()
            .filter(|node| node.enabled)
            .all(|node| node.state == NodeRuntimeState::Stopped));
    }

    #[test]
    fn runtime_node_commands_target_one_node() {
        let root = unique_test_dir("sim-rns-runtime-nodes");
        let project = create_project(&root, "Runtime Nodes").expect("project should be created");
        let runtime = FileBackedRuntime;
        runtime
            .execute(&project, RuntimeCommand::Boot)
            .expect("boot should succeed");

        let stopped = runtime
            .execute(
                &project,
                RuntimeCommand::StopNode {
                    element_id: "phone-a".to_string(),
                },
            )
            .expect("stop should succeed")
            .status;
        assert!(stopped
            .nodes
            .iter()
            .any(|node| node.element_id == "phone-a" && node.state == NodeRuntimeState::Stopped));
        assert!(
            stopped
                .nodes
                .iter()
                .any(|node| node.element_id == "backbone-a"
                    && node.state == NodeRuntimeState::Running)
        );

        let error = runtime
            .execute(
                &project,
                RuntimeCommand::StartNode {
                    element_id: "missing".to_string(),
                },
            )
            .expect_err("missing node should fail");
        assert!(matches!(error, RuntimeError::Validation(_)));
    }

    #[test]
    fn runtime_snapshots_are_metadata_markers() {
        let root = unique_test_dir("sim-rns-runtime-snapshots");
        let project =
            create_project(&root, "Runtime Snapshots").expect("project should be created");
        let runtime = FileBackedRuntime;
        runtime
            .execute(&project, RuntimeCommand::Boot)
            .expect("boot should succeed");
        let snapshot_status = runtime
            .execute(
                &project,
                RuntimeCommand::CreateSnapshot {
                    name: "booted".to_string(),
                    note: Some("metadata only".to_string()),
                },
            )
            .expect("snapshot should be created")
            .status;
        let snapshot_id = snapshot_status.snapshots[0].id.clone();

        runtime
            .execute(&project, RuntimeCommand::Shutdown)
            .expect("shutdown should succeed");
        let restored = runtime
            .execute(&project, RuntimeCommand::RestoreSnapshot { snapshot_id })
            .expect("restore should succeed")
            .status;

        assert_eq!(restored.vm_state, RuntimeVmState::Running);
        assert_eq!(restored.snapshots.len(), 1);
    }

    #[test]
    fn runtime_topology_overlay_does_not_edit_project_file() {
        let root = unique_test_dir("sim-rns-runtime-topology");
        let project = create_project(&root, "Runtime Topology").expect("project should be created");
        let runtime = FileBackedRuntime;
        let project_file_before =
            std::fs::read_to_string(project_file_path(&project.root_path)).expect("file exists");

        let status = runtime
            .execute(
                &project,
                RuntimeCommand::RemoveTopologyLink {
                    element_id: "phone-a".to_string(),
                    network_id: "lan-main".to_string(),
                },
            )
            .expect("remove should succeed")
            .status;
        let project_file_after =
            std::fs::read_to_string(project_file_path(&project.root_path)).expect("file exists");

        assert_eq!(project_file_before, project_file_after);
        assert!(!status
            .effective_topology
            .iter()
            .any(|link| link.element_id == "phone-a" && link.network_id == "lan-main"));
    }

    #[test]
    fn runtime_state_persists_across_instances() {
        let root = unique_test_dir("sim-rns-runtime-persist");
        let project = create_project(&root, "Runtime Persist").expect("project should be created");
        FileBackedRuntime
            .execute(&project, RuntimeCommand::Boot)
            .expect("boot should succeed");

        let status = FileBackedRuntime
            .status(&project)
            .expect("status should reload persisted state");

        assert_eq!(status.vm_state, RuntimeVmState::Running);
    }

    #[test]
    fn qemu_runtime_layout_is_project_local() {
        let root = unique_test_dir("sim-rns-qemu-layout");
        let project = create_project(&root, "QEMU Layout").expect("project should be created");
        let layout = QemuRuntime::default().layout(&project);

        assert_eq!(layout.runtime_dir, root.join(".sim-rns"));
        assert_eq!(layout.vm_dir, root.join(".sim-rns/vm"));
        assert_eq!(layout.snapshots_dir, root.join(".sim-rns/snapshots"));
        assert_eq!(layout.logs_dir, root.join(".sim-rns/logs"));
        assert_eq!(layout.disk_image_path, root.join(".sim-rns/vm/disk.qcow2"));
        assert_eq!(layout.pid_path, root.join(".sim-rns/vm/qemu.pid"));
        assert_eq!(layout.qmp_socket_path, root.join(".sim-rns/vm/qmp.sock"));
        assert_eq!(layout.log_path, root.join(".sim-rns/logs/qemu.log"));
    }

    #[test]
    fn qemu_runtime_status_reports_vm_assets_without_running_qemu() {
        let root = unique_test_dir("sim-rns-qemu-status");
        let project = create_project(&root, "QEMU Status").expect("project should be created");
        let runtime = QemuRuntime::default();
        let layout = runtime.layout(&project);
        std::fs::create_dir_all(&layout.vm_dir).expect("vm dir should be created");
        std::fs::write(&layout.disk_image_path, b"stub").expect("disk marker should be written");

        let status = runtime.status(&project).expect("status should load");

        assert_eq!(status.vm_state, RuntimeVmState::Stopped);
        assert_eq!(status.backend_state, RuntimeBackendState::Offline);
        assert!(status.vm_assets.prepared);
        assert_eq!(
            status.vm_assets.disk_image_path,
            layout.disk_image_path.to_string_lossy()
        );
    }

    #[test]
    fn qemu_boot_requires_a_project_disk_image() {
        let root = unique_test_dir("sim-rns-qemu-boot-missing");
        let project = create_project(&root, "QEMU Missing").expect("project should be created");
        let runtime = QemuRuntime::new("definitely-not-qemu");
        let error = runtime
            .execute(&project, RuntimeCommand::Boot)
            .expect_err("boot should fail before spawning without a disk image");

        assert!(matches!(error, RuntimeError::Validation(_)));
        assert!(runtime.layout(&project).vm_dir.is_dir());
        assert!(runtime.layout(&project).logs_dir.is_dir());
        assert!(runtime.layout(&project).snapshots_dir.is_dir());
    }

    #[test]
    fn qemu_prepare_vm_imports_source_image() {
        let root = unique_test_dir("sim-rns-qemu-prepare-import");
        let project = create_project(&root, "QEMU Import").expect("project should be created");
        let source_image = root.join("base.qcow2");
        std::fs::write(&source_image, b"base image bytes").expect("source should be written");
        let runtime = QemuRuntime::new("definitely-not-qemu");

        let status = runtime
            .execute(
                &project,
                RuntimeCommand::PrepareVm {
                    source_image: Some(source_image.to_string_lossy().into_owned()),
                    size_gb: 8,
                },
            )
            .expect("prepare should import source image")
            .status;

        let layout = runtime.layout(&project);
        assert!(status.vm_assets.prepared);
        assert_eq!(
            std::fs::read(&layout.disk_image_path).expect("disk should exist"),
            b"base image bytes"
        );
        assert!(layout.snapshots_dir.is_dir());
        assert!(layout.logs_dir.is_dir());
    }

    #[test]
    fn qemu_prepare_vm_refuses_to_overwrite_existing_disk() {
        let root = unique_test_dir("sim-rns-qemu-prepare-overwrite");
        let project = create_project(&root, "QEMU Overwrite").expect("project should be created");
        let runtime = QemuRuntime::new("definitely-not-qemu");
        let layout = runtime.layout(&project);
        std::fs::create_dir_all(&layout.vm_dir).expect("vm dir should be created");
        std::fs::write(&layout.disk_image_path, b"existing").expect("disk should be written");

        let error = runtime
            .execute(
                &project,
                RuntimeCommand::PrepareVm {
                    source_image: None,
                    size_gb: 8,
                },
            )
            .expect_err("prepare should not overwrite existing disk");

        assert!(matches!(error, RuntimeError::Validation(_)));
        assert_eq!(
            std::fs::read(&layout.disk_image_path).expect("disk should still exist"),
            b"existing"
        );
    }
}
