//! Host side of the versioned, project-local guest control channel.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::runtime::NodeRuntimeStatus;
use crate::{project_recipe, Attachment, Project, RuntimeError};

#[derive(Deserialize)]
pub(crate) struct GuestStatus {
    pub nodes: Vec<NodeRuntimeStatus>,
    pub topology: Vec<Attachment>,
    pub logs: BTreeMap<String, Vec<String>>,
}

pub(crate) fn request(socket: &Path, command: Value) -> Result<GuestStatus, RuntimeError> {
    let mut stream = UnixStream::connect(socket).map_err(error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(error)?;
    let id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(error)?
            .as_nanos()
    );
    let mut bytes =
        serde_json::to_vec(&json!({"version": 1, "id": id, "command": command})).map_err(error)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(error("guest request exceeds 16 MiB"));
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(error)?;
    let timeout = if matches!(command["op"].as_str(), Some("initialize" | "shutdown")) {
        20
    } else {
        3
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut reader = BufReader::new(stream);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| error("request timed out"))?;
        reader
            .get_ref()
            .set_read_timeout(Some(remaining))
            .map_err(error)?;
        let mut response = String::new();
        reader
            .by_ref()
            .take(1024 * 1024)
            .read_line(&mut response)
            .map_err(error)?;
        if !response.ends_with('\n') {
            return Err(error("incomplete or oversized guest response"));
        }
        let response: Value = serde_json::from_str(&response).map_err(error)?;
        if response["id"].as_str() != Some(&id) {
            continue;
        }
        if response["version"] != 1 {
            return Err(error("unsupported guest protocol"));
        }
        if let Some(message) = response.get("error") {
            return Err(error(message));
        }
        return serde_json::from_value(response["status"].clone()).map_err(error);
    }
}

pub(crate) fn provision(project: &Project, socket: &Path) -> Result<GuestStatus, RuntimeError> {
    let recipe = project_recipe(project).map_err(RuntimeError::ProjectLoad)?;
    let mut assets = BTreeMap::new();
    for element in &recipe.elements {
        for asset in &element.assets {
            let path = crate::resolve_project_relative_path(&project.root_path, &asset.source)
                .map_err(RuntimeError::Validation)?;
            let data = std::fs::read(path).map_err(error)?;
            if data.len() > 1024 * 1024 {
                return Err(error("individual guest asset exceeds 1 MiB"));
            }
            assets.insert(format!("{}/{}", element.id, asset.destination), data);
        }
    }
    request(
        socket,
        json!({"op": "initialize", "recipe": recipe, "assets": assets}),
    )
}

fn error(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Unavailable(format!("guest: {message}"))
}
