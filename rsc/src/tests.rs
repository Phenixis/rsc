use super::*;
use rsc::models::PlaylistId;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn keys_map_to_actions() {
    let none = KeyModifiers::NONE;
    assert_eq!(
        map_key(key(KeyCode::Char(' '), none)),
        Some(Action::TogglePause)
    );
    assert_eq!(map_key(key(KeyCode::Char('n'), none)), Some(Action::Next));
    assert_eq!(
        map_key(key(KeyCode::Char('p'), none)),
        Some(Action::Previous)
    );
    assert_eq!(map_key(key(KeyCode::Char('q'), none)), Some(Action::Quit));
    assert_eq!(
        map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(Action::Quit)
    );
    assert_eq!(map_key(key(KeyCode::Char('c'), none)), None);
    assert_eq!(map_key(key(KeyCode::Char('x'), none)), None);
}

#[test]
fn playlist_lookup_prefers_id_then_title_case_insensitively() {
    let playlists = vec![
        Playlist {
            id: PlaylistId("Rock".into()),
            title: "Rock".into(),
            track_count: 1,
        },
        Playlist {
            id: PlaylistId("jazz".into()),
            title: "Jazz".into(),
            track_count: 1,
        },
    ];
    assert_eq!(find_playlist(&playlists, "Rock").unwrap().title, "Rock");
    assert_eq!(find_playlist(&playlists, "JAZZ").unwrap().title, "Jazz");
    let err = find_playlist(&playlists, "metal").unwrap_err().to_string();
    assert!(err.contains("Rock, Jazz"), "{err}");
}
