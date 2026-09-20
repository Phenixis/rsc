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
