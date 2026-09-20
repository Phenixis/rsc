use crate::models::{Access, Track};

/// Ordered list of playable tracks with a cursor.
#[derive(Debug)]
pub struct Queue {
    tracks: Vec<Track>,
    cursor: usize,
    skipped: usize,
}

impl Queue {
    /// Blocked tracks are dropped here; `skipped()` reports how many.
    pub fn new(tracks: Vec<Track>) -> Self {
        let total = tracks.len();
        let tracks: Vec<Track> = tracks
            .into_iter()
            .filter(|t| t.access != Access::Blocked)
            .collect();
        Self {
            skipped: total - tracks.len(),
            tracks,
            cursor: 0,
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.tracks.get(self.cursor)
    }

    /// Moves to the next track. Returns `false` (cursor unchanged) on the last one.
    pub fn advance(&mut self) -> bool {
        if self.cursor + 1 < self.tracks.len() {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Moves to the previous track; on the first one the cursor stays (the track restarts).
    pub fn previous(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// 1-based position and length, for display.
    pub fn position(&self) -> (usize, usize) {
        (self.cursor + 1, self.tracks.len())
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }
}

#[cfg(test)]
mod tests;
