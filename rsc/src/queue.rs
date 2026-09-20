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
mod tests {
    use super::*;
    use crate::models::TrackId;

    fn track(id: &str, access: Access) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artist: "a".into(),
            permalink_url: None,
            duration_ms: None,
            access,
        }
    }

    fn ids(q: &Queue) -> Option<&str> {
        q.current().map(|t| t.id.0.as_str())
    }

    #[test]
    fn blocked_tracks_are_dropped_and_counted() {
        let q = Queue::new(vec![
            track("a", Access::Playable),
            track("b", Access::Blocked),
            track("c", Access::Preview),
        ]);
        assert_eq!(q.position(), (1, 2));
        assert_eq!(q.skipped(), 1);
    }

    #[test]
    fn advance_walks_in_order_and_stops_at_the_end() {
        let mut q = Queue::new(vec![
            track("a", Access::Playable),
            track("b", Access::Playable),
        ]);
        assert_eq!(ids(&q), Some("a"));
        assert!(q.advance());
        assert_eq!(ids(&q), Some("b"));
        assert!(!q.advance());
        assert_eq!(ids(&q), Some("b"));
    }

    #[test]
    fn previous_saturates_at_the_first_track() {
        let mut q = Queue::new(vec![
            track("a", Access::Playable),
            track("b", Access::Playable),
        ]);
        q.previous();
        assert_eq!(ids(&q), Some("a"));
        q.advance();
        q.previous();
        assert_eq!(ids(&q), Some("a"));
    }

    #[test]
    fn empty_queue_has_no_current_track() {
        let mut q = Queue::new(vec![track("a", Access::Blocked)]);
        assert!(q.is_empty());
        assert_eq!(q.current(), None);
        assert!(!q.advance());
    }
}
