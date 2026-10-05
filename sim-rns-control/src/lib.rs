//! Typed local control protocol shared by the GTK app and stdio MCP adapter.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
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
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        });
    root.join("sim-rns/control.sock")
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
    let mut stream = UnixStream::connect(path).map_err(|e| {
        format!(
            "cannot connect to app at {}: {e}; start sim-rns-app first",
            path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
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
