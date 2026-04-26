use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{project_recipe, Attachment, Project, Recipe};

const RUNTIME_SCHEMA_VERSION: u32 = 1;
const RUNTIME_DIR: &str = ".sim-rns";
const RUNTIME_STATE_FILE: &str = "runtime-state.json";

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
    pub nodes: Vec<NodeRuntimeStatus>,
    pub effective_topology: Vec<Attachment>,
    pub topology_overlay: RuntimeTopologyOverlay,
    pub snapshots: Vec<RuntimeSnapshot>,
    pub recent_events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeCommand {
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
        Ok(status_from_state(&state, &recipe))
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
            status: status_from_state(&state, &recipe),
        })
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

fn status_from_state(state: &RuntimeState, recipe: &Recipe) -> RuntimeStatus {
    RuntimeStatus {
        project_id: state.project_id.clone(),
        project_name: state.project_name.clone(),
        vm_state: state.vm_state.clone(),
        backend_state: state.backend_state.clone(),
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

fn unix_time_ms() -> Result<u64, RuntimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| RuntimeError::Persistence(format!("system clock error: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{
        FileBackedRuntime, NodeRuntimeState, ProjectRuntime, RuntimeCommand, RuntimeError,
        RuntimeVmState,
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
}
