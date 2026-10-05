//! Typed local control protocol shared by the GTK app and stdio MCP adapter.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const MAX_MESSAGE: u64 = 4 * 1024 * 1024;

fn object_schema(schema: &mut schemars::Schema) {
    schema.insert("type".into(), serde_json::json!("object"));
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "domain", content = "request", rename_all = "snake_case")]
pub enum Request {
    App(AppRequest),
    Project(ProjectRequest),
    Runtime(RuntimeRequest),
    Ui(UiRequest),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = object_schema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppRequest {
    State,
    Quit,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = object_schema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectRequest {
    /// Create and open a runnable two-node Reticulum demo using an existing guest bundle.
    CreateDemo {
        path: String,
        bundle: String,
    },
    Create {
        path: String,
        name: String,
        bundle: String,
    },
    Open {
        path: String,
    },
    Close,
    Inspect,
    AddNode,
    AddScript,
    ReadFile {
        path: String,
    },
    /// Replace an existing project source file. Paths are relative to the active project.
    WriteFile {
        path: String,
        contents: String,
    },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = object_schema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeRequest {
    Status,
    Prepare {
        source_image: Option<String>,
        #[serde(default)]
        size_gb: u32,
    },
    /// Prepare the disk if necessary, then boot the VM.
    Boot,
    Shutdown,
    Pause,
    Resume,
    StartNode {
        element_id: String,
    },
    StopNode {
        element_id: String,
    },
    RestartNode {
        element_id: String,
    },
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
    AddTopologyLink {
        element_id: String,
        network_id: String,
    },
    RemoveTopologyLink {
        element_id: String,
        network_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = object_schema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiRequest {
    /// Inspect windows, commands and widget IDs. IDs remain valid only while widgets exist.
    Tree,
    /// Capture a rendered app window as a PNG image.
    Screenshot {
        window_id: u64,
    },
    /// Invoke an enabled window action; discover exact names with tree.
    Command {
        window_id: u64,
        name: String,
    },
    /// Click a button or tab header using its normal callback.
    Activate {
        widget_id: u64,
    },
    /// Open a widget's secondary-click menu (including tab layout actions).
    ContextMenu {
        widget_id: u64,
    },
    DismissPopup {
        widget_id: u64,
    },
    /// Drive a drag controller; start coordinates are widget-local, offsets are pixels.
    Drag {
        widget_id: u64,
        start_x: f64,
        start_y: f64,
        offset_x: f64,
        offset_y: f64,
    },
    Focus {
        widget_id: u64,
    },
    SetText {
        widget_id: u64,
        text: String,
    },
    SetActive {
        widget_id: u64,
        active: bool,
    },
    SetValue {
        widget_id: u64,
        value: f64,
    },
    Select {
        widget_id: u64,
        index: u32,
    },
    Scroll {
        widget_id: u64,
        horizontal: f64,
        vertical: f64,
    },
    Present {
        window_id: u64,
    },
    Resize {
        window_id: u64,
        width: i32,
        height: i32,
    },
    Maximize {
        window_id: u64,
        maximized: bool,
    },
    CloseWindow {
        window_id: u64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
impl From<Result<serde_json::Value, String>> for Response {
    fn from(value: Result<serde_json::Value, String>) -> Self {
        match value {
            Ok(result) => Self {
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                result: None,
                error: Some(error),
            },
        }
    }
}

pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SIM_RNS_CONTROL_SOCKET") {
        return path.into();
    }
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(user_runtime_dir)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        });
    let preferred = root.join("sim-rns/control.sock");
    // Attach to an app started by an older, filtered MCP environment as well.
    let legacy = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".cache/sim-rns/control.sock");
    if !preferred.exists() && legacy.exists() {
        legacy
    } else {
        preferred
    }
}

pub fn read_message<T: serde::de::DeserializeOwned>(reader: impl Read) -> Result<T, String> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take(MAX_MESSAGE + 1))
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        return Err("control message exceeds limit or is not newline terminated".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub fn write_message(writer: &mut impl Write, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_MESSAGE {
        return Err("control message exceeds limit".into());
    }
    writer.write_all(&bytes).map_err(|e| e.to_string())
}

pub fn call(path: &std::path::Path, request: &Request) -> Result<serde_json::Value, String> {
    call_with_timeout(path, request, Duration::from_secs(180))
}

pub fn call_with_timeout(
    path: &std::path::Path,
    request: &Request,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let mut stream = UnixStream::connect(path).map_err(|e| {
        format!(
            "cannot connect to app at {}: {e}; start sim-rns-app first",
            path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    write_message(&mut stream, request)?;
    let response: Response = read_message(stream)?;
    match (response.result, response.error) {
        (_, Some(error)) => Err(error),
        (Some(result), None) => Ok(result),
        _ => Err("invalid empty response from app".into()),
    }
}

/// Linux login sessions keep their bus and display sockets here even when an MCP
/// host filters XDG_RUNTIME_DIR out of its child environment.
pub fn user_runtime_dir() -> Option<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let path = PathBuf::from(format!("/run/user/{uid}"));
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    (metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0).then_some(path)
}

/// Recover only desktop connection variables from an owned desktop-session process.
/// Explicit environment settings always win; never copy unrelated credentials.
pub fn desktop_environment() -> Result<std::collections::BTreeMap<String, String>, String> {
    const KEYS: &[&str] = &[
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ];
    let mut values: std::collections::BTreeMap<String, String> = KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| (key.to_string(), v))
        })
        .collect();
    if !values.contains_key("DISPLAY") && !values.contains_key("WAYLAND_DISPLAY") {
        let uid = unsafe { libc::geteuid() };
        let mut candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let path = entry.path();
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .chars()
                    .all(|c| c.is_ascii_digit())
                    || !std::fs::metadata(&path).is_ok_and(|m| m.uid() == uid)
                {
                    continue;
                }
                let name = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
                if ![
                    "i3",
                    "sway",
                    "Hyprland",
                    "gnome-shell",
                    "gnome-session-b",
                    "plasmashell",
                    "xfce4-session",
                    "cinnamon",
                    "mate-session",
                ]
                .contains(&name.trim())
                {
                    continue;
                }
                let Ok(bytes) = std::fs::read(path.join("environ")) else {
                    continue;
                };
                let candidate: std::collections::BTreeMap<String, String> = bytes
                    .split(|b| *b == 0)
                    .filter_map(|part| {
                        let text = std::str::from_utf8(part).ok()?;
                        let (key, value) = text.split_once('=')?;
                        (KEYS.contains(&key) && !value.is_empty())
                            .then(|| (key.to_string(), value.to_string()))
                    })
                    .collect();
                if candidate.contains_key("DISPLAY") || candidate.contains_key("WAYLAND_DISPLAY") {
                    candidates.push(candidate);
                }
            }
        }
        let displays: std::collections::BTreeSet<_> = candidates
            .iter()
            .map(|c| (c.get("DISPLAY"), c.get("WAYLAND_DISPLAY")))
            .collect();
        if displays.len() > 1 {
            return Err("Multiple desktop sessions found. Set DISPLAY or WAYLAND_DISPLAY in the MCP server environment to select one.".into());
        }
        if let Some(candidate) = candidates.into_iter().next() {
            for (key, value) in candidate {
                values.entry(key).or_insert(value);
            }
        }
    }
    if let Some(runtime) = user_runtime_dir() {
        values
            .entry("XDG_RUNTIME_DIR".into())
            .or_insert_with(|| runtime.to_string_lossy().into_owned());
    }
    if let Some(runtime) = values.get("XDG_RUNTIME_DIR") {
        let bus = PathBuf::from(runtime).join("bus");
        if bus.exists() {
            values
                .entry("DBUS_SESSION_BUS_ADDRESS".into())
                .or_insert_with(|| format!("unix:path={}", bus.display()));
        }
    }
    if !values.contains_key("DISPLAY") && !values.contains_key("WAYLAND_DISPLAY") {
        return Err("No desktop display found. Start sim-rns-app from your desktop, or configure DISPLAY / WAYLAND_DISPLAY for the MCP server.".into());
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_rejects_truncated_and_oversized_messages() {
        assert!(read_message::<Request>(&b"{}"[..]).is_err());
        assert!(read_message::<Request>(&vec![b'x'; MAX_MESSAGE as usize + 1][..]).is_err());
        let mut bytes = Vec::new();
        write_message(&mut bytes, &Request::App(AppRequest::State)).unwrap();
        assert!(matches!(
            read_message::<Request>(&bytes[..]).unwrap(),
            Request::App(AppRequest::State)
        ));
    }
    #[test]
    fn tools_have_object_schemas_and_reject_unexpected_arguments() {
        for schema in [
            schemars::schema_for!(AppRequest),
            schemars::schema_for!(ProjectRequest),
            schemars::schema_for!(RuntimeRequest),
            schemars::schema_for!(UiRequest),
        ] {
            assert_eq!(schema.get("type").unwrap(), "object");
        }
        assert!(serde_json::from_value::<UiRequest>(
            serde_json::json!({"operation":"activate","widget_id":1,"typo":true})
        )
        .is_err());
        assert!(
            serde_json::from_value::<UiRequest>(serde_json::json!({"operation":"activate"}))
                .is_err()
        );
    }
}
