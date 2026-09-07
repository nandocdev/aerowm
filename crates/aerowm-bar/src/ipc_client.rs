//! Minimal IPC client for the bar: one-shot queries plus a
//! newline-delimited event subscription stream.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use aerowm_ipc::{CompositorSnapshot, IpcCommand, IpcEvent, IpcResponse, ipc_socket_path};

fn send_raw(payload: &str) -> Result<UnixStream, String> {
    let socket_path = ipc_socket_path();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|e| format!("connect {}: {e}", socket_path.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("set timeout: {e}"))?;
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    stream.write_all(b"\n").map_err(|e| format!("write: {e}"))?;
    Ok(stream)
}

fn read_response(mut stream: &UnixStream) -> Result<IpcResponse, String> {
    let mut buf = String::new();
    stream
        .read_to_string(&mut buf)
        .map_err(|e| format!("read: {e}"))?;
    serde_json::from_str(&buf).map_err(|e| format!("parse response: {e}"))
}

/// Fire-and-forget command (used for click-to-switch-workspace).
pub fn send_command(cmd: &IpcCommand) {
    let payload = match serde_json::to_string(cmd) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("cannot serialize IPC command: {e}");
            return;
        }
    };
    match send_raw(&payload).and_then(|s| read_response(&s).map(|_| ())) {
        Ok(()) => {}
        Err(e) => log::warn!("IPC command failed: {e}"),
    }
}

/// Blocking snapshot query against the compositor.
pub fn query_state() -> Result<CompositorSnapshot, String> {
    let payload = serde_json::to_string(&IpcCommand::GetState).map_err(|e| e.to_string())?;
    let stream = send_raw(&payload)?;
    let response = read_response(&stream)?;
    if !response.success {
        return Err("compositor reported failure".to_string());
    }
    let message = response
        .message
        .ok_or_else(|| "empty snapshot response".to_string())?;
    serde_json::from_str(&message).map_err(|e| format!("parse snapshot: {e}"))
}

/// Live subscription: any event line means "state may have changed".
/// The caller re-queries via [`query_state`] (cheap, localhost).
pub struct Subscriber {
    reader: BufReader<UnixStream>,
    pending: String,
}

impl Subscriber {
    pub fn connect() -> Result<Self, String> {
        let payload =
            serde_json::to_string(&IpcCommand::Subscribe).map_err(|e| e.to_string())?;
        let stream = send_raw(&payload)?;
        stream
            .set_nonblocking(true)
            .map_err(|e| format!("nonblocking: {e}"))?;
        Ok(Self {
            reader: BufReader::new(stream),
            pending: String::new(),
        })
    }

    pub fn stream(&self) -> &UnixStream {
        self.reader.get_ref()
    }

    /// Drains complete event lines. Returns how many were seen.
    pub fn drain_events(&mut self) -> usize {
        let mut count = 0;
        loop {
            self.pending.clear();
            match self.reader.read_line(&mut self.pending) {
                Ok(0) => break, // WouldBlock on nonblocking socket
                Ok(_) => {
                    let line = self.pending.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<IpcEvent>(line) {
                        Ok(_) => count += 1,
                        Err(e) => log::debug!("ignoring non-event line: {e}"),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::warn!("subscribe read error: {e}");
                    break;
                }
            }
        }
        count
    }
}
