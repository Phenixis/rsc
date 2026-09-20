//! End-to-end: the real `rsc` binary drives the real mpv, and we listen to what comes out.
//!
//! mpv writes its audio output to a WAV file (`--ao=pcm`) instead of a sound card, so this
//! runs anywhere, sound hardware or not, and faster than real time. We then look for the
//! test tones in that file: it proves the right files were decoded and played, once each,
//! in order. What happens after mpv hands samples to the OS audio stack is out of scope.

#[macro_use]
mod common;

use std::path::Path;
use std::process::Command;

use common::{SAMPLE_RATE, tone_runs, write_sine_wav};

const LOW: f32 = 440.0;
const MID: f32 = 660.0;
const HIGH: f32 = 880.0;

/// Runs `rsc play <playlist>` with mpv capturing to `out`; returns the captured samples.
fn play_and_capture(library: &Path, playlist: &str, out: &Path, runtime_dir: &Path) -> Vec<i16> {
    let output = Command::new(env!("CARGO_BIN_EXE_rsc"))
        .arg("--library")
        .arg(library)
        // Raw + append: mpv's WAV writer truncates the file whenever it reopens the audio
        // output between two tracks, which would randomly lose the first ones.
        .args(["--mpv-arg=--ao=pcm", "--mpv-arg=--ao-pcm-waveheader=no"])
        .args([
            "--mpv-arg=--ao-pcm-append=yes",
            "--mpv-arg=--audio-channels=mono",
        ])
        .args(["--mpv-arg=--audio-format=s16"])
        .arg(format!("--mpv-arg=--audio-samplerate={SAMPLE_RATE}"))
        .arg(format!("--mpv-arg=--ao-pcm-file={}", out.display()))
        .args(["play", playlist])
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .output() // stdin is null and stdout is a pipe: rsc runs without a terminal
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "rsc failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Playlist finished."), "{stdout}");
    common::read_raw_samples(out)
}

#[test]
fn a_playlist_is_heard_in_order_each_track_once_and_in_full() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let playlist = dir.path().join("library/mix");
    std::fs::create_dir_all(&playlist).unwrap();
    // File names give the order; frequencies are deliberately not ascending by name.
    write_sine_wav(&playlist.join("01-first.wav"), 1.0, MID);
    write_sine_wav(&playlist.join("02-second.wav"), 1.0, HIGH);
    write_sine_wav(&playlist.join("03-third.wav"), 1.0, LOW);

    let samples = play_and_capture(
        &dir.path().join("library"),
        "mix",
        &dir.path().join("out.pcm"),
        dir.path(),
    );

    let runs = tone_runs(&samples, &[LOW, MID, HIGH]);
    let order: Vec<f32> = runs.iter().map(|(f, _)| *f).collect();
    assert_eq!(order, [MID, HIGH, LOW], "runs: {runs:?}");
    // 1 s = ten 100 ms windows per track; a little slack for encoder delay at the edges.
    for (freq, windows) in &runs {
        assert!(
            (9..=11).contains(windows),
            "{freq} Hz was heard for {windows} windows: {runs:?}"
        );
    }
}

#[test]
fn a_single_track_playlist_plays_and_ends() {
    require_mpv!();
    let dir = tempfile::tempdir().unwrap();
    let playlist = dir.path().join("library/solo");
    std::fs::create_dir_all(&playlist).unwrap();
    write_sine_wav(&playlist.join("only.wav"), 0.5, HIGH);

    let samples = play_and_capture(
        &dir.path().join("library"),
        "solo",
        &dir.path().join("out.pcm"),
        dir.path(),
    );
    assert_eq!(tone_runs(&samples, &[LOW, MID, HIGH]).len(), 1);
}

// --- the "ear" itself: check the analysis on synthetic signals before trusting it -------------

fn sine(freq: f32, seconds: f32) -> Vec<i16> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.wav");
    write_sine_wav(&path, seconds, freq);
    common::read_wav_samples(&path)
}

#[test]
fn the_ear_orders_tones_and_ignores_silence() {
    let mut signal = sine(LOW, 0.5);
    signal.extend(vec![0i16; SAMPLE_RATE as usize / 2]); // half a second of silence
    signal.extend(sine(HIGH, 0.5));
    signal.extend(sine(MID, 0.3));

    assert_eq!(
        tone_runs(&signal, &[LOW, MID, HIGH]),
        [(LOW, 5), (HIGH, 5), (MID, 3)]
    );
}

#[test]
fn the_ear_hears_nothing_in_silence() {
    assert!(tone_runs(&vec![0i16; SAMPLE_RATE as usize], &[LOW, MID, HIGH]).is_empty());
}
