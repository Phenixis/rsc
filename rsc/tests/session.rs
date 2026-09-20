//! Session behaviour against a scripted backend and a `FakePlayer`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use async_trait::async_trait;
use rsc::backend::Backend;
use rsc::models::{Access, Playlist, PlaylistId, StreamTarget, Track, TrackId};
use rsc::player::fake::Call;
use rsc::player::{EndReason, FakePlayer, PlayerEvent};
use rsc::session::{Flow, Session};

#[derive(Default)]
struct StubBackend {
    unresolvable: HashSet<String>,
    resolved: Mutex<Vec<String>>,
}

#[async_trait]
impl Backend for StubBackend {
    async fn login(&self) -> Result<()> {
        Ok(())
    }
    async fn my_playlists(&self) -> Result<Vec<Playlist>> {
        Ok(vec![])
    }
    async fn playlist_tracks(&self, _: &PlaylistId) -> Result<Vec<Track>> {
        Ok(vec![])
    }
    async fn stream_target(&self, track: &Track) -> Result<StreamTarget> {
        self.resolved.lock().unwrap().push(track.id.0.clone());
        if self.unresolvable.contains(&track.id.0) {
            bail!("gone");
        }
        Ok(StreamTarget {
            url: format!("stub://{}", track.id.0),
            headers: vec![],
        })
    }
}

fn track(id: &str, access: Access) -> Track {
    Track {
        id: TrackId(id.into()),
        title: id.into(),
        artist: "artist".into(),
        permalink_url: None,
        duration_ms: None,
        access,
    }
}

fn session(
    backend: StubBackend,
    tracks: Vec<Track>,
) -> (Session, Arc<StubBackend>, Arc<FakePlayer>) {
    let backend = Arc::new(backend);
    let player = Arc::new(FakePlayer::default());
    let session = Session::new(backend.clone(), player.clone(), tracks);
    (session, backend, player)
}

const EOF: PlayerEvent = PlayerEvent::EndFile(EndReason::Eof);
const STOP: PlayerEvent = PlayerEvent::EndFile(EndReason::Stop);

#[tokio::test]
async fn plays_in_order_and_skips_the_blocked_track() {
    let tracks = vec![
        track("a", Access::Playable),
        track("b", Access::Blocked),
        track("c", Access::Playable),
    ];
    let (mut s, _, player) = session(StubBackend::default(), tracks);

    assert_eq!(s.start().await.unwrap(), Flow::Continue);
    assert_eq!(s.handle_event(EOF).await.unwrap(), Flow::Continue);
    assert_eq!(s.handle_event(EOF).await.unwrap(), Flow::Finished);
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://c"]);
}

#[tokio::test]
async fn end_file_stop_does_not_advance() {
    let tracks = ["a", "b", "c"]
        .map(|id| track(id, Access::Playable))
        .to_vec();
    let (mut s, _, player) = session(StubBackend::default(), tracks);

    s.start().await.unwrap();
    s.next().await.unwrap();
    // mpv reports the replaced track "a" as stopped: must not skip "b".
    assert_eq!(s.handle_event(STOP).await.unwrap(), Flow::Continue);
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://b"]);

    s.handle_event(EOF).await.unwrap();
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://b", "stub://c"]);
}

#[tokio::test]
async fn manual_navigation_is_bounded() {
    let tracks = ["a", "b"].map(|id| track(id, Access::Playable)).to_vec();
    let (mut s, _, player) = session(StubBackend::default(), tracks);

    s.start().await.unwrap();
    s.previous().await.unwrap(); // first track: restarts it
    s.next().await.unwrap();
    s.next().await.unwrap(); // last track: no-op
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://a", "stub://b"]);
}

#[tokio::test]
async fn streams_are_resolved_lazily_and_unresolvable_tracks_are_skipped() {
    let backend = StubBackend {
        unresolvable: HashSet::from(["b".to_string()]),
        ..StubBackend::default()
    };
    let tracks = ["a", "b", "c"]
        .map(|id| track(id, Access::Playable))
        .to_vec();
    let (mut s, backend, player) = session(backend, tracks);

    s.start().await.unwrap();
    assert_eq!(
        *backend.resolved.lock().unwrap(),
        ["a"],
        "only the first track so far"
    );

    s.handle_event(EOF).await.unwrap();
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://c"]);
    assert_eq!(*backend.resolved.lock().unwrap(), ["a", "b", "c"]);
}

#[tokio::test]
async fn a_track_that_errors_in_the_player_is_skipped() {
    let tracks = ["a", "b"].map(|id| track(id, Access::Playable)).to_vec();
    let (mut s, _, player) = session(StubBackend::default(), tracks);

    s.start().await.unwrap();
    s.handle_event(PlayerEvent::EndFile(EndReason::Error))
        .await
        .unwrap();
    assert_eq!(player.loaded_urls(), ["stub://a", "stub://b"]);
}

#[tokio::test]
async fn pause_toggles_and_resets_on_track_change() {
    let tracks = ["a", "b"].map(|id| track(id, Access::Playable)).to_vec();
    let (mut s, _, player) = session(StubBackend::default(), tracks);

    s.start().await.unwrap();
    s.toggle_pause().await.unwrap();
    s.next().await.unwrap(); // mpv keeps `pause` across files, so we unpause explicitly
    s.toggle_pause().await.unwrap();

    let pauses: Vec<_> = player
        .calls()
        .into_iter()
        .filter_map(|c| {
            if let Call::SetPause(p) = c {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(pauses, [true, false, true]);
}

#[tokio::test]
async fn nothing_playable_finishes_immediately() {
    let (mut s, _, player) = session(StubBackend::default(), vec![track("a", Access::Blocked)]);
    assert_eq!(s.start().await.unwrap(), Flow::Finished);
    assert!(player.calls().is_empty());
}
