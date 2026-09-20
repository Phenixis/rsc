mod local;

pub use local::LocalBackend;

use anyhow::Result;
use async_trait::async_trait;

use crate::models::{Playlist, PlaylistId, StreamTarget, Track};

/// Where playlists and audio come from: the official API, a local folder, a mock…
#[async_trait]
pub trait Backend: Send + Sync {
    async fn login(&self) -> Result<()>;
    async fn my_playlists(&self) -> Result<Vec<Playlist>>;
    async fn playlist_tracks(&self, id: &PlaylistId) -> Result<Vec<Track>>;
    /// Called just before each track is played, never when the playlist is loaded:
    /// real stream URLs are short-lived.
    async fn stream_target(&self, track: &Track) -> Result<StreamTarget>;
}
