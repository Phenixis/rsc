//! Real mpv, real IPC socket, no audio output (`--ao=null`). Skipped if mpv is missing.

use std::path::Path;
use std::time::Duration;

use rsc::models::StreamTarget;
use rsc::player::{EndReason, MpvOptions, MpvPlayer, Player, PlayerEvent};
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

fn mpv_available() -> bool {
    std::process::Command::new("mpv")
        .arg("--version")
        .output()
        .is_ok()
}

macro_rules! require_mpv {
    () => {
        if !mpv_available() {
            eprintln!("mpv not installed, skipping");
            return;
        }
    };
}

/// Mono 8 kHz 16-bit silence.
fn write_wav(path: &Path, seconds: f32) {
    let rate = 8000u32;
    let data_len = (rate as f32 * seconds) as u32 * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    b.resize(b.len() + data_len as usize, 0);
    std::fs::write(path, b).unwrap();
}

fn target(path: &Path) -> StreamTarget {
    StreamTarget {
        url: path.to_string_lossy().into_owned(),
        headers: vec![],
    }
}

async fn spawn() -> (MpvPlayer, Receiver<PlayerEvent>) {
    MpvPlayer::spawn(MpvOptions {
        extra_args: vec!["--ao=null".into()],
        ..MpvOptions::default()
    })
    .await
    .unwrap()
}

/// Next event matching `pick`, ignoring the others; fails after 10 s.
async fn wait_for<T>(
    rx: &mut Receiver<PlayerEvent>,
    pick: impl Fn(&PlayerEvent) -> Option<T>,
) -> T {
    timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("mpv exited");
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    })
    .await
    .expect("timed out waiting for a player event")
}

fn end_reason(event: &PlayerEvent) -> Option<EndReason> {
    match event {
        PlayerEvent::EndFile(reason) => Some(*reason),
        _ => None,
    }
}

#[tokio::test]
async fn a_finished_track_reports_eof() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 0.3);
    let (player, mut rx) = spawn().await;

    player.load(&target(&wav)).await.unwrap();
    assert_eq!(wait_for(&mut rx, end_reason).await, EndReason::Eof);
    player.shutdown().await;
}

#[tokio::test]
async fn replacing_a_track_reports_stop_then_eof() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let (long, short) = (dir.path().join("long.wav"), dir.path().join("short.wav"));
    write_wav(&long, 30.0);
    write_wav(&short, 0.3);
    let (player, mut rx) = spawn().await;

    player.load(&target(&long)).await.unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, PlayerEvent::Duration(_)).then_some(())
    })
    .await;
    player.load(&target(&short)).await.unwrap();

    // The interrupted track must not look like a natural end.
    assert_eq!(wait_for(&mut rx, end_reason).await, EndReason::Stop);
    assert_eq!(wait_for(&mut rx, end_reason).await, EndReason::Eof);
    player.shutdown().await;
}

#[tokio::test]
async fn pause_is_reported_back() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("long.wav");
    write_wav(&wav, 30.0);
    let (player, mut rx) = spawn().await;

    player.load(&target(&wav)).await.unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, PlayerEvent::Duration(_)).then_some(())
    })
    .await;
    player.set_pause(true).await.unwrap();
    wait_for(&mut rx, |e| (*e == PlayerEvent::Paused(true)).then_some(())).await;
    player.shutdown().await;
}

#[tokio::test]
async fn a_missing_file_reports_an_error_not_an_eof() {
    require_mpv!();
    let (player, mut rx) = spawn().await;

    player
        .load(&target(Path::new("/nonexistent/x.mp3")))
        .await
        .unwrap();
    assert_eq!(wait_for(&mut rx, end_reason).await, EndReason::Error);
    player.shutdown().await;
}

#[tokio::test]
async fn shutdown_leaves_no_process_and_no_socket() {
    require_mpv!();
    let (player, _rx) = spawn().await;
    let pid = player.pid().expect("pid");
    let socket = player.socket_path().to_path_buf();
    assert!(socket.exists());
    assert!(Path::new(&format!("/proc/{pid}")).exists());

    player.shutdown().await;
    assert!(!socket.exists());
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "mpv is still there"
    );
}

#[tokio::test]
async fn dropping_the_player_kills_mpv() {
    require_mpv!();
    let (player, _rx) = spawn().await;
    let pid = player.pid().expect("pid");
    let socket = player.socket_path().to_path_buf();

    drop(player);
    assert!(!socket.exists());
    timeout(Duration::from_secs(5), async {
        while Path::new(&format!("/proc/{pid}")).exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("mpv outlived its player");
}

#[tokio::test]
async fn a_missing_binary_gives_a_clear_error() {
    let err = MpvPlayer::spawn(MpvOptions {
        binary: "definitely-not-mpv".into(),
        ..MpvOptions::default()
    })
    .await
    .err()
    .expect("should fail")
    .to_string();
    assert!(err.contains("not found in PATH"), "{err}");
}
