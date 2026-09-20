//! Local-folder backend: every sub-directory of the library root is a playlist and
//! every audio file inside it (sorted by name) is a track. No network involved.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::fs;

use super::Backend;
use crate::models::{Access, Playlist, PlaylistId, StreamTarget, Track, TrackId};

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"];

pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        // mpv is a child process: hand it absolute paths, whatever our cwd is.
        let root = std::path::absolute(&root).unwrap_or(root);
        Self { root }
    }
}

/// Joins a backend-provided id onto the root, refusing anything that could escape it.
fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if relative.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        bail!("invalid id {relative:?}");
    }
    Ok(root.join(rel))
}

fn is_audio(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(ext)))
}

/// Visible, UTF-8 entries of `dir` as `(name, is_dir)`, sorted by name.
async fn sorted_entries(dir: &Path) -> Result<Vec<(String, bool)>> {
    let mut read = fs::read_dir(dir)
        .await
        .with_context(|| format!("cannot read {}", dir.display()))?;
    let mut entries = Vec::new();
    while let Some(entry) = read.next_entry().await? {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        // `fs::metadata` follows symlinks, unlike `DirEntry::file_type`.
        let is_dir = fs::metadata(entry.path()).await?.is_dir();
        entries.push((name, is_dir));
    }
    entries.sort();
    Ok(entries)
}

async fn audio_files(dir: &Path) -> Result<Vec<String>> {
    Ok(sorted_entries(dir)
        .await?
        .into_iter()
        .filter(|(name, is_dir)| !is_dir && is_audio(name))
        .map(|(name, _)| name)
        .collect())
}

#[async_trait]
impl Backend for LocalBackend {
    async fn login(&self) -> Result<()> {
        Ok(())
    }

    async fn my_playlists(&self) -> Result<Vec<Playlist>> {
        let mut playlists = Vec::new();
        for (name, is_dir) in sorted_entries(&self.root).await? {
            if !is_dir {
                continue;
            }
            let track_count = audio_files(&self.root.join(&name)).await?.len();
            playlists.push(Playlist {
                id: PlaylistId(name.clone()),
                title: name,
                track_count,
            });
        }
        Ok(playlists)
    }

    async fn playlist_tracks(&self, id: &PlaylistId) -> Result<Vec<Track>> {
        let dir = safe_join(&self.root, &id.0)?;
        Ok(audio_files(&dir)
            .await?
            .into_iter()
            .map(|file| Track {
                title: Path::new(&file)
                    .file_stem()
                    .map_or_else(|| file.clone(), |s| s.to_string_lossy().into_owned()),
                id: TrackId(format!("{}/{file}", id.0)),
                artist: "local".into(),
                permalink_url: None,
                duration_ms: None,
                access: Access::Playable,
            })
            .collect())
    }

    async fn stream_target(&self, track: &Track) -> Result<StreamTarget> {
        let path = safe_join(&self.root, &track.id.0)?;
        fs::metadata(&path)
            .await
            .with_context(|| format!("cannot open {}", path.display()))?;
        Ok(StreamTarget {
            url: path.to_string_lossy().into_owned(),
            headers: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests;
