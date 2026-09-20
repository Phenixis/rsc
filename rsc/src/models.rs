//! Domain types shared by every backend.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlaylistId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub title: String,
    pub track_count: usize,
}

/// Whether a track can be streamed outside the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Playable,
    /// Excerpt only; still played, never extended.
    Preview,
    /// No stream available: skipped when building the queue.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artist: String,
    pub permalink_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub access: Access,
}

/// What the player needs to start a track. Resolved lazily, right before playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamTarget {
    pub url: String,
    /// Extra HTTP headers the stream endpoint requires (e.g. `Authorization`).
    pub headers: Vec<(String, String)>,
}
