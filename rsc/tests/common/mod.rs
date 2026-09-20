//! Helpers shared by the integration tests (each test file includes this module).
#![allow(dead_code)] // every test crate uses a different subset

use std::path::Path;

/// Set `RSC_REQUIRE_E2E=1` (CI, pre-push) to turn "mpv is missing" from a silent skip
/// into a failure: a skipped test must never look like a green one there.
pub fn skip_or_fail(what: &str) {
    if std::env::var_os("RSC_REQUIRE_E2E").is_some_and(|v| !v.is_empty() && v != "0") {
        panic!("{what} is required (RSC_REQUIRE_E2E is set) but was not found");
    }
    eprintln!("{what} not installed, skipping");
}

pub fn mpv_available() -> bool {
    std::process::Command::new("mpv")
        .arg("--version")
        .output()
        .is_ok()
}

macro_rules! require_mpv {
    () => {
        if !common::mpv_available() {
            common::skip_or_fail("mpv");
            return;
        }
    };
}

pub const SAMPLE_RATE: u32 = 8000;

/// Mono 16-bit WAV at [`SAMPLE_RATE`]: a sine at `freq` Hz, or silence when `None`.
pub fn write_wav(path: &Path, seconds: f32, freq: Option<f32>) {
    let count = (SAMPLE_RATE as f32 * seconds) as u32;
    let data_len = count * 2;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for n in 0..count {
        let sample = freq.map_or(0.0, |f| {
            0.5 * (2.0 * std::f32::consts::PI * f * n as f32 / SAMPLE_RATE as f32).sin()
        });
        b.extend_from_slice(&((sample * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

pub fn write_silence_wav(path: &Path, seconds: f32) {
    write_wav(path, seconds, None);
}

pub fn write_sine_wav(path: &Path, seconds: f32, freq: f32) {
    write_wav(path, seconds, Some(freq));
}

/// Samples of a 16-bit mono WAV file (walks the RIFF chunks to find `data`).
pub fn read_wav_samples(path: &Path) -> Vec<i16> {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(bytes.len() > 12 && &bytes[..4] == b"RIFF", "not a WAV file");
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if id == b"data" {
            // A streaming writer may leave the size unset (0 or 0xFFFFFFFF): take what is there.
            let end = if size == 0 || body + size > bytes.len() {
                bytes.len()
            } else {
                body + size
            };
            return bytes[body..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes(*c))
                .collect();
        }
        at = body + size + (size & 1);
    }
    panic!("no data chunk in {}", path.display());
}

/// Samples of a headerless 16-bit little-endian mono file (`--ao-pcm-waveheader=no`).
pub fn read_raw_samples(path: &Path) -> Vec<i16> {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

/// Goertzel algorithm: signal energy at one frequency, normalised by the window length.
fn goertzel(window: &[i16], freq: f32) -> f32 {
    let coeff = 2.0 * (2.0 * std::f32::consts::PI * freq / SAMPLE_RATE as f32).cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in window {
        let s = f32::from(x) + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / window.len() as f32
}

/// Which of `candidates` is playing in each 100 ms window, with silence skipped, folded into
/// runs: `(frequency, number of windows)`. This is how a test "hears" what was played.
pub fn tone_runs(samples: &[i16], candidates: &[f32]) -> Vec<(f32, usize)> {
    const WINDOW: usize = (SAMPLE_RATE / 10) as usize;
    const SILENCE_RMS: f32 = 500.0; // out of 32768; our test tones sit around 11000
    let mut runs: Vec<(f32, usize)> = Vec::new();
    for window in samples.as_chunks::<WINDOW>().0 {
        let window = &window[..];
        let rms =
            (window.iter().map(|&x| f32::from(x).powi(2)).sum::<f32>() / WINDOW as f32).sqrt();
        if rms < SILENCE_RMS {
            continue;
        }
        let best = candidates
            .iter()
            .copied()
            .max_by(|a, b| goertzel(window, *a).total_cmp(&goertzel(window, *b)))
            .expect("at least one candidate");
        match runs.last_mut() {
            Some((freq, count)) if *freq == best => *count += 1,
            _ => runs.push((best, 1)),
        }
    }
    runs
}

/// PIDs of processes whose command line mentions `needle` (e.g. a temp directory).
pub fn processes_mentioning(needle: &str) -> Vec<u32> {
    let me = std::process::id();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me {
            continue;
        }
        if let Ok(cmdline) = std::fs::read(entry.path().join("cmdline"))
            && String::from_utf8_lossy(&cmdline).contains(needle)
        {
            found.push(pid);
        }
    }
    found
}

/// `*.sock` files left in `dir` (rsc puts its mpv socket in `$XDG_RUNTIME_DIR`).
pub fn leftover_sockets(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".sock"))
        .collect()
}
