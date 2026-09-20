pub mod fake;
pub mod mpv;

use anyhow::Result;
use async_trait::async_trait;

use crate::models::StreamTarget;

pub use fake::FakePlayer;
pub use mpv::{MpvOptions, MpvPlayer};

#[async_trait]
pub trait Player: Send + Sync {
    /// Replaces whatever is playing with `target`.
    async fn load(&self, target: &StreamTarget) -> Result<()>;
    async fn set_pause(&self, paused: bool) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

/// Why playback of a file ended. Only `Eof` means "the track finished by itself":
/// treating every end as an EOF makes manual skips advance twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// Reached the end of the file.
    Eof,
    /// Stopped by a command (`stop`, or replaced by a new `loadfile`).
    Stop,
    Quit,
    /// The file could not be played (network error, bad format…).
    Error,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    EndFile(EndReason),
    /// Seconds.
    Position(f64),
    /// Seconds.
    Duration(f64),
    Paused(bool),
}
