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
