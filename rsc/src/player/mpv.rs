//! mpv controlled through its JSON IPC socket
//! (<https://mpv.io/manual/stable/#json-ipc>): newline-delimited JSON over a unix socket.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

use super::{EndReason, Player, PlayerEvent};
use crate::models::StreamTarget;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct MpvOptions {
    pub binary: String,
    /// Appended to the command line, e.g. `--ao=null` for headless runs.
    pub extra_args: Vec<String>,
}

impl Default for MpvOptions {
    fn default() -> Self {
        Self {
            binary: "mpv".into(),
            extra_args: Vec::new(),
        }
    }
}

pub struct MpvPlayer {
    writer: Mutex<OwnedWriteHalf>,
    next_request_id: AtomicU64,
    socket_path: PathBuf,
    child: Mutex<Child>,
    pid: Option<u32>,
    reader: JoinHandle<()>,
}

impl MpvPlayer {
    /// Starts an idle mpv and connects to it. Events come out of the returned receiver;
    /// it closes when mpv exits.
    pub async fn spawn(options: MpvOptions) -> Result<(Self, mpsc::Receiver<PlayerEvent>)> {
        let socket_path = socket_path();
        let mut child = Command::new(&options.binary)
            .args([
                "--idle=yes",
                "--no-video",
                "--no-terminal",
                "--really-quiet",
            ])
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .args(&options.extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Safety net: if we die without calling `shutdown`, mpv must not outlive us.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => anyhow!(
                    "`{}` not found in PATH: rsc needs mpv to play audio \
                     (e.g. `sudo apt install mpv`)",
                    options.binary
                ),
                _ => anyhow!(e).context(format!("failed to start `{}`", options.binary)),
            })?;
        let pid = child.id();

        let stream = connect(&socket_path, &mut child).await?;
        let (read_half, write_half) = stream.into_split();
        let (tx, rx) = mpsc::channel(64);
        let player = Self {
            writer: Mutex::new(write_half),
            next_request_id: AtomicU64::new(1),
            socket_path,
            child: Mutex::new(child),
            pid,
            reader: tokio::spawn(read_events(read_half, tx)),
        };
        for (id, property) in [(1, "time-pos"), (2, "duration"), (3, "pause")] {
            player
                .send(json!(["observe_property", id, property]))
                .await?;
        }
        Ok((player, rx))
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Asks mpv to quit, waits for it (killing it after 2 s) and removes the socket.
    pub async fn shutdown(&self) {
        let _ = self.send(json!(["quit"])).await;
        let mut child = self.child.lock().await;
        if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }

    async fn send(&self, command: Value) -> Result<()> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let mut line =
            serde_json::to_vec(&json!({ "command": command, "request_id": request_id }))?;
        line.push(b'\n');
        self.writer
            .lock()
            .await
            .write_all(&line)
            .await
            .context("cannot talk to mpv (did it exit?)")
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[async_trait]
impl Player for MpvPlayer {
    async fn load(&self, target: &StreamTarget) -> Result<()> {
        self.send(loadfile_command(target)).await
    }

    async fn set_pause(&self, paused: bool) -> Result<()> {
        self.send(json!(["set_property", "pause", paused])).await
    }

    async fn stop(&self) -> Result<()> {
        self.send(json!(["stop"])).await
    }
}

fn socket_path() -> PathBuf {
    static INSTANCE: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let n = INSTANCE.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("rsc-{}-{n}.sock", std::process::id()))
}

/// The socket appears asynchronously after mpv starts: retry until it does, but give
/// up early if mpv died (bad option, missing library…).
async fn connect(path: &Path, child: &mut Child) -> Result<UnixStream> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                if let Some(status) = child.try_wait()? {
                    bail!("mpv exited during startup ({status})");
                }
                if Instant::now() >= deadline {
                    bail!("timed out connecting to mpv at {}: {err}", path.display());
                }
                sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

async fn read_events(read_half: OwnedReadHalf, tx: mpsc::Sender<PlayerEvent>) {
    let mut lines = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match parse_message(&line) {
            // Progress ticks are frequent and disposable; everything else must arrive.
            Some(Message::Event(event @ PlayerEvent::Position(_))) => {
                let _ = tx.try_send(event);
            }
            Some(Message::Event(event)) => {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            Some(Message::Reply { error }) if error != "success" => {
                tracing::warn!("mpv command failed: {error}");
            }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Message {
    Event(PlayerEvent),
    /// Answer to one of our commands.
    Reply {
        error: String,
    },
}

fn parse_message(line: &str) -> Option<Message> {
    let value: Value = serde_json::from_str(line).ok()?;
    let Some(event) = value.get("event").and_then(Value::as_str) else {
        let error = value.get("error")?.as_str()?;
        return Some(Message::Reply {
            error: error.to_owned(),
        });
    };
    match event {
        "end-file" => {
            let reason = value.get("reason").and_then(Value::as_str).unwrap_or("");
            Some(Message::Event(PlayerEvent::EndFile(end_reason(reason))))
        }
        "property-change" => {
            let data = value.get("data")?;
            let event = match value.get("name")?.as_str()? {
                "time-pos" => PlayerEvent::Position(data.as_f64()?),
                "duration" => PlayerEvent::Duration(data.as_f64()?),
                "pause" => PlayerEvent::Paused(data.as_bool()?),
                _ => return None,
            };
            Some(Message::Event(event))
        }
        _ => None,
    }
}

fn end_reason(reason: &str) -> EndReason {
    match reason {
        "eof" => EndReason::Eof,
        "stop" => EndReason::Stop,
        "quit" => EndReason::Quit,
        "error" => EndReason::Error,
        _ => EndReason::Other,
    }
}

fn loadfile_command(target: &StreamTarget) -> Value {
    if target.headers.is_empty() {
        return json!(["loadfile", target.url, "replace"]);
    }
    // `http-header-fields` is a comma-separated list, so header values must not
    // contain commas. The `-1` is the (mpv >= 0.38) insertion index preceding options.
    let fields = target
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(",");
    json!(["loadfile", target.url, "replace", -1, { "http-header-fields": fields }])
}

#[cfg(test)]
mod tests;
