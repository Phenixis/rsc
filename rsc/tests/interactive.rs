//! The interactive terminal, for real: `rsc` runs inside a pseudo-terminal, a test types
//! keys and reads what appears on screen, like a person would.

#[macro_use]
mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use expectrl::{Eof, Session};

/// Three 30 s silent tracks: long enough that nothing ends by itself while a test types.
fn library(dir: &Path) {
    let playlist = dir.join("library/mix");
    std::fs::create_dir_all(&playlist).unwrap();
    for n in 1..=3 {
        common::write_silence_wav(&playlist.join(format!("0{n}-track.wav")), 30.0);
    }
}

fn rsc_command(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rsc"));
    cmd.arg("--library")
        .arg(dir.join("library"))
        .args(["--mpv-arg=--ao=null", "play", "mix"])
        .env("XDG_RUNTIME_DIR", dir);
    cmd
}

fn session(cmd: Command) -> Session {
    let mut session = Session::spawn(cmd).expect("cannot start a pseudo-terminal session");
    session.set_expect_timeout(Some(Duration::from_secs(15)));
    session
}

fn assert_nothing_left_behind(dir: &Path) {
    assert_eq!(
        common::leftover_sockets(dir),
        Vec::<String>::new(),
        "socket left behind"
    );
    let needle = dir.to_string_lossy().into_owned();
    let start = std::time::Instant::now();
    while !common::processes_mentioning(&needle).is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "mpv is still running"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn keys_drive_the_player_and_q_quits_cleanly() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    library(dir.path());
    let mut p = session(rsc_command(dir.path()));

    p.expect("1/3").unwrap();
    p.expect("[q]uit").unwrap(); // the key legend is shown
    p.send("n").unwrap();
    p.expect("2/3").unwrap();
    p.send("n").unwrap();
    p.expect("3/3").unwrap();
    p.send("p").unwrap();
    p.expect("2/3").unwrap();
    p.send(" ").unwrap();
    p.expect("paused").unwrap();
    p.send(" ").unwrap();
    p.expect("resumed").unwrap();

    p.send("q").unwrap();
    p.expect(Eof).unwrap();
    assert_nothing_left_behind(dir.path());
}

#[test]
fn ctrl_c_is_a_key_in_raw_mode_and_quits_cleanly() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    library(dir.path());
    let mut p = session(rsc_command(dir.path()));

    p.expect("1/3").unwrap();
    p.send("\x03").unwrap(); // Ctrl-C
    p.expect(Eof).unwrap();
    assert_nothing_left_behind(dir.path());
}

/// Raw mode must be undone on the way out, or the user's shell is left unusable.
/// `stty -g` prints every terminal setting in one line: it must be identical before and after.
#[test]
fn the_terminal_is_restored_after_quitting() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    library(dir.path());
    let rsc = rsc_command(dir.path());
    let script = format!(
        "echo BEFORE:$(stty -g); {:?} {}; echo AFTER:$(stty -g)",
        rsc.get_program(),
        rsc.get_args()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut cmd = Command::new("bash");
    cmd.args(["-c", &script]).env("XDG_RUNTIME_DIR", dir.path());
    let mut p = session(cmd);

    // `expect` consumes what it scans: keep both halves of the output (BEFORE: comes first).
    let mut screen = String::from_utf8_lossy(p.expect("[q]uit").unwrap().before()).into_owned();
    p.send("q").unwrap();
    screen.push_str(&String::from_utf8_lossy(p.expect(Eof).unwrap().as_bytes()));

    let setting = |label: &str| {
        let line = screen
            .lines()
            .find(|l| l.contains(label))
            .unwrap_or_else(|| panic!("no {label} line in {screen:?}"));
        line.split(label).nth(1).unwrap().trim().to_owned()
    };
    let (before, after) = (setting("BEFORE:"), setting("AFTER:"));
    assert!(!before.is_empty());
    assert_eq!(before, after, "the terminal was left in a different mode");
}
