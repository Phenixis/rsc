//! A running `rsc` that is told to stop must leave nothing behind: no mpv, no socket.
//! (Closing the terminal window sends SIGHUP; `kill` sends SIGTERM; Ctrl-C is SIGINT.)

#[macro_use]
mod common;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn wait_until(what: &str, deadline: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn start_rsc(runtime_dir: &Path) -> Child {
    let playlist = runtime_dir.join("library/long");
    std::fs::create_dir_all(&playlist).unwrap();
    common::write_silence_wav(&playlist.join("track.wav"), 60.0);
    Command::new(env!("CARGO_BIN_EXE_rsc"))
        .arg("--library")
        .arg(runtime_dir.join("library"))
        .args(["--mpv-arg=--ao=null", "play", "long"])
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn assert_clean_exit_on(signal: &str) {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let mut rsc = start_rsc(dir.path());
    let dir_str = dir.path().to_string_lossy().into_owned();

    // mpv is up once its socket exists and it shows in the process list.
    wait_until("mpv to start", Duration::from_secs(10), || {
        !common::leftover_sockets(dir.path()).is_empty()
            && !common::processes_mentioning(&dir_str).is_empty()
    });

    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(rsc.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());

    let start = Instant::now();
    let exit = loop {
        if let Some(exit) = rsc.try_wait().unwrap() {
            break exit;
        }
        if start.elapsed() > Duration::from_secs(10) {
            rsc.kill().ok();
            panic!("rsc did not exit after SIG{signal}");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(exit.success(), "SIG{signal}: rsc exited with {exit}");
    assert_eq!(
        common::leftover_sockets(dir.path()),
        Vec::<String>::new(),
        "SIG{signal}: socket left behind"
    );
    wait_until("mpv to be gone", Duration::from_secs(5), || {
        common::processes_mentioning(&dir_str).is_empty()
    });
}

#[test]
fn sigint_shuts_everything_down() {
    assert_clean_exit_on("INT");
}

#[test]
fn sigterm_shuts_everything_down() {
    assert_clean_exit_on("TERM");
}

#[test]
fn sighup_when_the_terminal_closes_shuts_everything_down() {
    assert_clean_exit_on("HUP");
}
