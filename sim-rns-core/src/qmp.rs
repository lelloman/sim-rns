//! Bounded, framed QMP exchanges. Events are never mistaken for command replies.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::RuntimeError;

pub(crate) struct Qmp {
    reader: BufReader<UnixStream>,
    next_id: u64,
    deadline: Instant,
}

impl Qmp {
    pub(crate) fn connect(path: &Path) -> Result<Self, RuntimeError> {
        let stream = UnixStream::connect(path).map_err(unavailable)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(unavailable)?;
        let mut client = Self {
            reader: BufReader::new(stream),
            next_id: 0,
            deadline: Instant::now() + Duration::from_secs(2),
        };
        if client.read()?.get("QMP").is_none() {
            return Err(RuntimeError::Unavailable("invalid QMP greeting".into()));
        }
        client.execute("qmp_capabilities")?;
        Ok(client)
    }

    pub(crate) fn execute(&mut self, command: &str) -> Result<Value, RuntimeError> {
        self.next_id += 1;
        let id = self.next_id;
        let mut payload =
            serde_json::to_vec(&json!({"execute": command, "id": id})).map_err(unavailable)?;
        payload.push(b'\n');
        self.reader
            .get_mut()
            .write_all(&payload)
            .map_err(unavailable)?;
        loop {
            let reply = self.read()?;
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(RuntimeError::Unavailable(format!("QMP {command}: {error}")));
            }
            return reply.get("return").cloned().ok_or_else(|| {
                RuntimeError::Unavailable(format!("QMP {command}: missing result"))
            });
        }
    }

    fn read(&mut self) -> Result<Value, RuntimeError> {
        let mut line = Vec::new();
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| RuntimeError::Unavailable("QMP exchange timed out".into()))?;
            self.reader
                .get_ref()
                .set_read_timeout(Some(remaining))
                .map_err(unavailable)?;
            let chunk = self.reader.fill_buf().map_err(unavailable)?;
            if chunk.is_empty() {
                return Err(RuntimeError::Unavailable(
                    "QMP disconnected before replying".into(),
                ));
            }
            let count = chunk
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(chunk.len(), |n| n + 1);
            line.extend_from_slice(&chunk[..count]);
            self.reader.consume(count);
            if line.len() > 1024 * 1024 {
                return Err(RuntimeError::Unavailable("QMP reply exceeds 1 MiB".into()));
            }
            if line.last() == Some(&b'\n') {
                return serde_json::from_slice(&line).map_err(unavailable);
            }
        }
    }
}

fn unavailable(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Unavailable(format!("QMP: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn fragmented_replies_and_events_do_not_hide_command_errors() {
        let path = std::env::temp_dir().join(format!("sim-qmp-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            reader.get_mut().write_all(b"{\"QMP\":{}}\r\n").unwrap();
            for (id, result) in [
                (1, json!({"return":{}})),
                (2, json!({"error":{"desc":"rejected"}})),
            ] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["id"], id);
                reader
                    .get_mut()
                    .write_all(b"{\"event\":\"STOP\"}\r\n")
                    .unwrap();
                let mut result = result;
                result["id"] = id.into();
                let bytes = format!("{result}\r\n");
                for chunk in bytes.as_bytes().chunks(3) {
                    reader.get_mut().write_all(chunk).unwrap();
                }
            }
        });
        let mut qmp = Qmp::connect(&path).unwrap();
        assert!(qmp
            .execute("stop")
            .unwrap_err()
            .to_string()
            .contains("rejected"));
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
