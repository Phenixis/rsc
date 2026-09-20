use super::*;

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"").unwrap();
}

#[tokio::test]
async fn folders_are_playlists_and_files_are_sorted_tracks() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join("mix/02-b.mp3"));
    touch(&dir.path().join("mix/01-a.MP3"));
    touch(&dir.path().join("mix/cover.jpg"));
    touch(&dir.path().join("mix/.hidden.mp3"));
    touch(&dir.path().join("empty/readme.txt"));
    touch(&dir.path().join("loose.mp3"));
    let backend = LocalBackend::new(dir.path());

    let playlists = backend.my_playlists().await.unwrap();
    let summary: Vec<_> = playlists
        .iter()
        .map(|p| (p.title.as_str(), p.track_count))
        .collect();
    assert_eq!(summary, [("empty", 0), ("mix", 2)]);

    let tracks = backend
        .playlist_tracks(&PlaylistId("mix".into()))
        .await
        .unwrap();
    let ids: Vec<_> = tracks.iter().map(|t| t.id.0.as_str()).collect();
    assert_eq!(ids, ["mix/01-a.MP3", "mix/02-b.mp3"]);
    assert_eq!(tracks[0].title, "01-a");
}

#[tokio::test]
async fn stream_target_is_an_absolute_existing_path() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join("mix/a.mp3"));
    let backend = LocalBackend::new(dir.path());
    let tracks = backend
        .playlist_tracks(&PlaylistId("mix".into()))
        .await
        .unwrap();

    let target = backend.stream_target(&tracks[0]).await.unwrap();
    assert!(Path::new(&target.url).is_absolute());
    assert!(target.url.ends_with("mix/a.mp3"));

    std::fs::remove_file(&target.url).unwrap();
    assert!(backend.stream_target(&tracks[0]).await.is_err());
}

#[tokio::test]
async fn ids_cannot_escape_the_library_root() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalBackend::new(dir.path());
    for bad in ["../etc", "/etc", "a/../..", ""] {
        assert!(
            backend
                .playlist_tracks(&PlaylistId(bad.into()))
                .await
                .is_err(),
            "{bad:?} should be rejected"
        );
    }
}
