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
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn tokens(n: u32) -> Tokens {
        Tokens {
            access_token: format!("access-{n}"),
            refresh_token: format!("refresh-{n}"),
            expires_at: 1000 + u64::from(n),
        }
    }

    fn store_in(dir: &tempfile::TempDir) -> TokenStore {
        TokenStore::new(dir.path().join("data/rsc/tokens.json"))
    }

    #[tokio::test]
    async fn loading_without_a_file_is_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(store_in(&dir).load().await.unwrap(), None);
    }

    #[tokio::test]
    async fn saved_tokens_are_loaded_back_and_missing_folders_are_created() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&tokens(1)).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tokens(1)));
    }

    #[tokio::test]
    async fn saving_again_replaces_the_previous_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&tokens(1)).await.unwrap();
        store.save(&tokens(2)).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tokens(2)));
    }

    #[tokio::test]
    async fn the_file_is_only_readable_by_its_owner() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&tokens(1)).await.unwrap();
        store.save(&tokens(2)).await.unwrap();
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&tokens(1)).await.unwrap();
        let files: Vec<_> = std::fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(files, ["tokens.json"]);
    }

    #[tokio::test]
    async fn a_stale_temporary_file_from_a_crash_is_ignored_then_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.save(&tokens(1)).await.unwrap();
        // Simulates dying halfway through a previous save.
        let tmp = store.path().with_extension("json.tmp");
        std::fs::write(&tmp, b"{ truncated").unwrap();

        assert_eq!(store.load().await.unwrap(), Some(tokens(1)));
        store.save(&tokens(2)).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tokens(2)));
        assert!(!tmp.exists());
    }

    #[tokio::test]
    async fn a_corrupted_file_is_an_error_that_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), b"not json").unwrap();

        let err = store.load().await.unwrap_err();
        assert!(format!("{err:#}").contains("tokens.json"), "{err:#}");
    }
}
