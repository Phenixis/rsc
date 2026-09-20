//! Token file persistence. Losing a refresh token logs the user out for good (they are
//! single-use), so writes are atomic and durable: tmp file -> fsync -> rename.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use super::tokens::Tokens;

pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Ok(None)` when there is no file yet; an error when it exists but is unreadable.
    pub async fn load(&self) -> Result<Option<Tokens>> {
        match fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .with_context(|| format!("{} is not a valid token file", self.path.display())),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| format!("cannot read {}", self.path.display())),
        }
    }

    pub async fn save(&self, tokens: &Tokens) -> Result<()> {
        let dir = match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        fs::create_dir_all(dir).await?;

        let tmp = self.path.with_extension("json.tmp");
        // A leftover from a crash could have looser permissions: start from nothing.
        match fs::remove_file(&tmp).await {
            Err(err) if err.kind() != ErrorKind::NotFound => return Err(err.into()),
            _ => {}
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600) // from creation: the secrets are never readable by others
            .open(&tmp)
            .await
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        file.write_all(&serde_json::to_vec_pretty(tokens)?).await?;
        file.sync_all().await?; // data on disk before the rename makes it visible
        drop(file);

        fs::rename(&tmp, &self.path).await?;
        fs::File::open(dir).await?.sync_all().await?; // persist the rename itself
        Ok(())
    }
}

#[cfg(test)]
mod tests;
