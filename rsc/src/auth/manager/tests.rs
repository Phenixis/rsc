use std::fs::File;
use std::os::unix::fs::PermissionsExt;

use serde_json::json;
use tokio::task::JoinSet;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use super::*;
use crate::test_support::{Fixture, NOW, token_response_json, tokens};

const REDIRECT: &str = "http://127.0.0.1:8888/callback";

// --- the happy paths ----------------------------------------------------------------------

#[tokio::test]
async fn a_valid_token_is_returned_without_any_network_call() {
    let f = Fixture::with_tokens(tokens("access-1", "refresh-1", NOW + 1000)).await;

    assert_eq!(f.auth.get_valid_token().await.unwrap(), "access-1");
    assert_eq!(f.total_requests().await, 0);
}

#[tokio::test]
async fn an_expired_token_is_refreshed_with_the_stored_refresh_token() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    f.mount_refresh_ok("new-access", "new-refresh").await;

    assert_eq!(f.auth.get_valid_token().await.unwrap(), "new-access");
    assert_eq!(f.refresh_tokens_sent().await, ["old-refresh"]);
}

#[tokio::test]
async fn refreshed_tokens_are_saved_with_an_expiry_computed_from_the_clock() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    f.mount_refresh_ok("new-access", "new-refresh").await;

    f.auth.get_valid_token().await.unwrap();
    // expires_in 3600, minus the 60 s safety margin
    assert_eq!(
        f.stored().await,
        Some(tokens("new-access", "new-refresh", NOW + 3600 - 60))
    );
}

#[tokio::test]
async fn a_token_that_expires_later_is_refreshed_once_the_clock_passes_it() {
    let f = Fixture::with_tokens(tokens("a1", "r1", NOW + 100)).await;
    f.mount_refresh_ok("a2", "r2").await;

    assert_eq!(f.auth.get_valid_token().await.unwrap(), "a1");
    f.clock.set(NOW + 100);
    assert_eq!(f.auth.get_valid_token().await.unwrap(), "a2");
    assert_eq!(f.auth.get_valid_token().await.unwrap(), "a2"); // and now it is valid again
    assert_eq!(f.total_requests().await, 1);
}

// --- refresh tokens are single-use: the classic ways to lose a login -----------------------

/// The server burns a refresh token on first use, so a second refresh with the same
/// token is an `invalid_grant`. Eight callers hit an expired token at once: exactly one
/// may go to the network, the others must wait and reuse its result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_cause_exactly_one_refresh() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    f.mount_single_use_refresh().await;

    let mut callers = JoinSet::new();
    for _ in 0..8 {
        let auth = f.auth.clone();
        callers.spawn(async move { auth.get_valid_token().await });
    }
    while let Some(result) = callers.join_next().await {
        assert_eq!(result.unwrap().unwrap(), "new-access");
    }
    assert_eq!(f.refresh_tokens_sent().await, ["old-refresh"]);
}

#[tokio::test]
async fn a_refused_refresh_token_means_the_user_must_log_in_again() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    f.mount_token_endpoint(
        ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
    )
    .await;

    let err = f.auth.get_valid_token().await.unwrap_err();
    assert!(matches!(err, AuthError::SessionExpired), "{err:?}");
    assert_eq!(
        f.stored().await,
        Some(tokens("old-access", "old-refresh", NOW - 1)),
        "file must be untouched"
    );
}

#[tokio::test]
async fn a_server_error_is_not_a_logout_and_the_next_attempt_can_succeed() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    f.mount_refresh_ok("new-access", "new-refresh").await;

    let err = f.auth.get_valid_token().await.unwrap_err();
    assert!(
        matches!(err, AuthError::Token(TokenError::Status(503))),
        "{err:?}"
    );
    assert_eq!(
        f.stored().await,
        Some(tokens("old-access", "old-refresh", NOW - 1)),
        "refresh token kept"
    );

    assert_eq!(f.auth.get_valid_token().await.unwrap(), "new-access");
}

/// The new refresh token exists nowhere else: if it cannot be written, pretending
/// everything is fine would silently log the user out on the next run.
#[tokio::test]
async fn if_the_new_tokens_cannot_be_saved_the_call_fails_and_says_where() {
    let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
    f.mount_refresh_ok("new-access", "new-refresh").await;
    let dir = f.store_path.parent().unwrap().to_owned();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap(); // reading works, writing does not
    let restore =
        || std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if File::create(dir.join("probe")).is_ok() {
        restore();
        eprintln!("directory permissions are not enforced here (running as root?), skipping");
        return;
    }

    let err = f.auth.get_valid_token().await.unwrap_err();
    restore();
    assert!(matches!(err, AuthError::Storage(_)), "{err:?}");
    assert!(err.to_string().contains("tokens.json"), "{err}");
}

// --- not logged in / broken files ---------------------------------------------------------

#[tokio::test]
async fn without_stored_tokens_the_user_must_log_in() {
    let f = Fixture::new().await;

    let err = f.auth.get_valid_token().await.unwrap_err();
    assert!(matches!(err, AuthError::NotLoggedIn), "{err:?}");
    assert_eq!(f.total_requests().await, 0);
}

#[tokio::test]
async fn a_corrupted_token_file_is_a_storage_problem_not_a_silent_logout() {
    let f = Fixture::new().await;
    std::fs::write(&f.store_path, b"{ not json").unwrap();

    let err = f.auth.get_valid_token().await.unwrap_err();
    assert!(matches!(err, AuthError::Storage(_)), "{err:?}");
    assert!(err.to_string().contains("tokens.json"), "{err}");
}

// --- after a 401 from the API -------------------------------------------------------------

#[tokio::test]
async fn after_a_401_the_rejected_token_is_refreshed_even_if_it_looks_valid() {
    let f = Fixture::with_tokens(tokens("access-1", "refresh-1", NOW + 1000)).await;
    f.mount_refresh_ok("access-2", "refresh-2").await;

    assert_eq!(
        f.auth.refresh_after_unauthorized("access-1").await.unwrap(),
        "access-2"
    );
    assert_eq!(f.refresh_tokens_sent().await, ["refresh-1"]);
}

#[tokio::test]
async fn after_a_401_a_token_somebody_else_already_refreshed_is_reused() {
    // The caller was rejected with access-1, but by now access-2 is stored.
    let f = Fixture::with_tokens(tokens("access-2", "refresh-2", NOW + 1000)).await;

    assert_eq!(
        f.auth.refresh_after_unauthorized("access-1").await.unwrap(),
        "access-2"
    );
    assert_eq!(f.total_requests().await, 0);
}

// --- login ---------------------------------------------------------------------------------

#[tokio::test]
async fn completing_a_login_exchanges_the_code_and_saves_the_tokens() {
    let f = Fixture::new().await;
    f.mount_token_endpoint(
        ResponseTemplate::new(200).set_body_json(token_response_json("a1", "r1", 3600)),
    )
    .await;

    f.auth
        .complete_login("CODE", "VERIFIER", REDIRECT)
        .await
        .unwrap();

    assert_eq!(f.stored().await, Some(tokens("a1", "r1", NOW + 3600 - 60)));
    let form = f.last_token_request_form().await;
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(
        (form["code"].as_str(), form["code_verifier"].as_str()),
        ("CODE", "VERIFIER")
    );
    assert_eq!(form["redirect_uri"], REDIRECT);
}

#[tokio::test]
async fn a_failed_login_leaves_the_existing_tokens_untouched() {
    let f = Fixture::with_tokens(tokens("keep-access", "keep-refresh", NOW + 1000)).await;
    f.mount_token_endpoint(
        ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
    )
    .await;

    let err = f
        .auth
        .complete_login("BAD", "V", REDIRECT)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AuthError::Token(TokenError::InvalidGrant)),
        "{err:?}"
    );
    assert_eq!(
        f.stored().await,
        Some(tokens("keep-access", "keep-refresh", NOW + 1000))
    );
}
