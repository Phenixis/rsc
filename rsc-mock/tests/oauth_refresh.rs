//! `grant_type=refresh_token`: single-use tokens, rotation, clock, concurrency.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::*;
use rsc_mock::{MockConfig, TokenKind};

fn change_last_char(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    let last = chars.pop().expect("non-empty");
    chars.push(if last == 'a' { 'b' } else { 'a' });
    chars.into_iter().collect()
}

// spec: B45
#[tokio::test]
async fn a_refresh_returns_a_new_pair_and_kills_the_old_refresh_token() {
    let h = start().await;
    let t1 = h.session(1).await;

    let resp = h.refresh(t1.refresh()).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(header(&resp, "content-type").as_deref(), Some(JSON_UTF8));
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    let t2 = read_tokens(resp).await;

    let mut keys: Vec<&str> = t2
        .raw
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "access_token",
            "expires_in",
            "refresh_token",
            "scope",
            "token_type"
        ]
    );
    assert_eq!(t2.expires_in, 3599);
    assert_eq!(t2.raw["scope"], serde_json::json!(""));
    assert_eq!(t2.raw["token_type"], serde_json::json!("bearer"));
    assert!(is_opaque(&t2.access_token));
    assert!(is_opaque(t2.refresh()));
    assert_ne!(t2.access_token, t1.access_token);
    assert_ne!(
        t2.refresh(),
        t1.refresh(),
        "each success returns a NEW refresh token"
    );

    // The old one is spent.
    assert_sc_error(h.refresh(t1.refresh()).await, 400, "invalid_grant").await;
    // The new one works.
    h.refresh_ok(t2.refresh()).await;
}

// spec: B45
#[tokio::test]
async fn a_replayed_refresh_token_is_invalid_grant_however_often_it_is_replayed() {
    let h = start().await;
    let t1 = h.session(1).await;
    h.refresh_ok(t1.refresh()).await;
    for _ in 0..3 {
        assert_sc_error(h.refresh(t1.refresh()).await, 400, "invalid_grant").await;
    }
}

// spec: B46
#[tokio::test]
async fn rotation_over_many_rounds_never_reuses_or_revives_a_token() {
    let h = start().await;
    let mut current = h.session(1).await;
    let mut spent: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(current.access_token.clone());
    seen.insert(current.refresh().to_owned());

    for round in 0..12 {
        let next = h.refresh_ok(current.refresh()).await;
        assert!(
            seen.insert(next.access_token.clone()),
            "round {round}: access token reused"
        );
        assert!(
            seen.insert(next.refresh().to_owned()),
            "round {round}: refresh token reused"
        );
        spent.push(current.refresh().to_owned());
        current = next;
        // Every refresh token spent so far stays spent.
        for old in &spent {
            assert_sc_error(h.refresh(old).await, 400, "invalid_grant").await;
        }
    }
    assert_eq!(spent.len(), 12);
    // The last token is still good.
    h.refresh_ok(current.refresh()).await;
}

// spec: B47
#[tokio::test]
async fn the_old_access_token_stays_valid_until_its_own_expiry() {
    let h = start().await;
    let t1 = h.session(1).await;
    h.server.advance_clock(Duration::from_secs(1000));
    let t2 = h.refresh_ok(t1.refresh()).await;

    let old = h
        .server
        .token_info(&t1.access_token)
        .expect("old token still valid");
    let new = h
        .server
        .token_info(&t2.access_token)
        .expect("new token valid");
    assert_eq!(old.remaining, Duration::from_secs(2599));
    assert_eq!(new.remaining, Duration::from_secs(3599));

    h.server.advance_clock(Duration::from_secs(2599));
    assert!(
        h.server.token_info(&t1.access_token).is_none(),
        "expired on schedule"
    );
    let new = h
        .server
        .token_info(&t2.access_token)
        .expect("new token still valid");
    assert_eq!(new.remaining, Duration::from_secs(1000));
}

// spec: B48
#[tokio::test]
async fn a_replay_does_not_revoke_the_newer_tokens() {
    let h = start().await;
    let t1 = h.session(1).await;
    let t2 = h.refresh_ok(t1.refresh()).await;

    assert_sc_error(h.refresh(t1.refresh()).await, 400, "invalid_grant").await;

    assert!(h.server.token_info(&t2.access_token).is_some());
    assert!(h.server.token_info(&t1.access_token).is_some());
    let t3 = h.refresh_ok(t2.refresh()).await;
    assert!(h.server.token_info(&t3.access_token).is_some());
}

// spec: B49
#[tokio::test]
async fn refresh_works_after_the_access_token_expired() {
    let h = start().await;
    let t1 = h.session(1).await;
    h.server.advance_clock(Duration::from_secs(3599));
    assert!(h.server.token_info(&t1.access_token).is_none());

    let t2 = h.refresh_ok(t1.refresh()).await;
    let info = h.server.token_info(&t2.access_token).expect("fresh token");
    assert_eq!(info.kind, TokenKind::User);
    assert_eq!(info.remaining, Duration::from_secs(3599));
}

// spec: B49
#[tokio::test]
async fn refresh_tokens_do_not_expire() {
    let h = start().await;
    let mut current = h.session(1).await;
    for days in [1u64, 30, 90] {
        h.server.advance_clock(Duration::from_secs(86_400 * days));
        current = h.refresh_ok(current.refresh()).await;
        assert!(h.server.token_info(&current.access_token).is_some());
    }
}

// spec: B49
#[tokio::test]
async fn short_lived_tokens_follow_a_configured_lifetime_across_refreshes() {
    let config = MockConfig {
        access_token_lifetime: Duration::from_secs(20),
        ..MockConfig::default()
    };
    let h = start_with(config).await;
    let mut current = h.session(1).await;
    assert_eq!(current.expires_in, 20);
    for _ in 0..5 {
        h.server.advance_clock(Duration::from_secs(19));
        assert_eq!(
            h.server
                .token_info(&current.access_token)
                .unwrap()
                .remaining,
            Duration::from_secs(1)
        );
        h.server.advance_clock(Duration::from_secs(1));
        assert!(h.server.token_info(&current.access_token).is_none());
        current = h.refresh_ok(current.refresh()).await;
        assert_eq!(current.expires_in, 20);
        assert_eq!(
            h.server
                .token_info(&current.access_token)
                .unwrap()
                .remaining,
            Duration::from_secs(20)
        );
    }
}

// spec: B50
#[tokio::test]
async fn redirect_uri_is_optional_on_refresh_and_ignored_when_given() {
    let h = start().await;
    let mut current = h.session(1).await;
    // Without (what `rsc` sends), with the registered one, with garbage, with scope.
    let extras: [&[(&str, &str)]; 4] = [
        &[],
        &[("redirect_uri", REDIRECT_URI)],
        &[("redirect_uri", "http://elsewhere.example/nope")],
        &[("scope", "ignored"), ("unknown", "x")],
    ];
    for extra in extras {
        let refresh = current.refresh().to_owned();
        let mut fields = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ];
        fields.extend_from_slice(extra);
        current = read_tokens(h.token(&fields).await).await;
    }
}

// spec: B50
#[tokio::test]
async fn a_missing_or_empty_refresh_token_is_an_invalid_request() {
    let h = start().await;
    let t1 = h.session(1).await;
    assert_sc_error(
        h.token(&[("grant_type", "refresh_token")]).await,
        400,
        "invalid_request",
    )
    .await;
    assert_sc_error(
        h.token(&[("grant_type", "refresh_token"), ("refresh_token", "")])
            .await,
        400,
        "invalid_request",
    )
    .await;
    let resp = h
        .token_basic(CLIENT_ID, CLIENT_SECRET, &[("grant_type", "refresh_token")])
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;
    // Nothing consumed.
    h.refresh_ok(t1.refresh()).await;
}

// spec: B50
#[tokio::test]
async fn unknown_refresh_tokens_are_invalid_grant() {
    let h = start().await;
    let t1 = h.session(1).await;
    let login = h.login(2).await;
    let neighbour = change_last_char(t1.refresh());
    let padded = format!("{} ", t1.refresh());
    let lowercase = t1.refresh().to_lowercase();
    let long = "r".repeat(30 * 1024);
    let forty = "r".repeat(40);
    for bad in [
        "nope",
        "0",
        forty.as_str(),
        neighbour.as_str(),
        padded.as_str(),
        long.as_str(),
        login.code.as_str(),
    ] {
        assert_sc_error(h.refresh(bad).await, 400, "invalid_grant").await;
    }
    if lowercase != t1.refresh() {
        assert_sc_error(h.refresh(&lowercase).await, 400, "invalid_grant").await;
    }
    // The real one survived every attempt.
    h.refresh_ok(t1.refresh()).await;
}

// spec: B51
#[tokio::test]
async fn a_refresh_token_belongs_to_its_client() {
    let h = start_with(two_client_config()).await;
    let t1 = h.session(1).await;

    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", t1.refresh()),
    ];
    // Other client, authenticated with its own credentials, in both forms.
    let mut body_form = vec![
        ("client_id", "other-client-id"),
        ("client_secret", "other-client-secret"),
    ];
    body_form.extend_from_slice(&form);
    assert_sc_error(h.token_raw(&body_form).await, 400, "invalid_grant").await;
    let resp = h
        .token_basic("other-client-id", "other-client-secret", &form)
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;

    // The owner can still use it: the foreign attempts did not burn it.
    let t2 = h.refresh_ok(t1.refresh()).await;
    assert_eq!(
        h.server.token_info(&t2.access_token).unwrap().client_id,
        CLIENT_ID
    );
}

// spec: B51
#[tokio::test]
async fn a_failed_client_authentication_does_not_consume_the_refresh_token() {
    let h = start().await;
    let t1 = h.session(1).await;

    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", t1.refresh()),
    ];
    let mut bad = vec![("client_id", CLIENT_ID), ("client_secret", "wrong")];
    bad.extend_from_slice(&form);
    assert_sc_error(h.token_raw(&bad).await, 401, "invalid_client").await;
    assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
    let resp = h.token_basic(CLIENT_ID, "wrong", &form).await;
    assert_sc_error(resp, 401, "invalid_client").await;

    h.refresh_ok(t1.refresh()).await;
}

// spec: B51
#[tokio::test]
async fn refresh_accepts_http_basic_authentication() {
    let h = start().await;
    let t1 = h.session(1).await;
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", t1.refresh()),
    ];
    let resp = h.token_basic(CLIENT_ID, CLIENT_SECRET, &form).await;
    let t2 = read_tokens(resp).await;
    assert_ne!(t2.refresh(), t1.refresh());
    assert_sc_error(h.refresh(t1.refresh()).await, 400, "invalid_grant").await;
}

// spec: B52
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_refreshes_of_one_token_yield_exactly_one_success() {
    let h = start().await;
    let mut current = h.session(1).await;

    for round in 0..5 {
        let old_refresh = current.refresh().to_owned();
        let results = h
            .race(
                32,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", old_refresh.as_str()),
                ],
            )
            .await;
        let winners: Vec<&(u16, String)> = results.iter().filter(|(s, _)| *s == 200).collect();
        assert_eq!(
            winners.len(),
            1,
            "round {round}: exactly one winner, got {results:?}"
        );
        for (status, body) in results.iter().filter(|(s, _)| *s != 200) {
            assert_eq!(*status, 400, "round {round}");
            assert!(body.contains("invalid_grant"), "{body}");
        }

        let body: serde_json::Value = serde_json::from_str(&winners[0].1).unwrap();
        let next_access = body["access_token"].as_str().unwrap().to_owned();
        let next_refresh = body["refresh_token"].as_str().unwrap().to_owned();
        assert_ne!(next_refresh, old_refresh);
        assert!(h.server.token_info(&next_access).is_some());

        // The losers did not poison the winner's new token (B48), and the old one is dead.
        assert_sc_error(h.refresh(&old_refresh).await, 400, "invalid_grant").await;
        current = h.refresh_ok(&next_refresh).await;
    }
}

// spec: B52
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_refreshes_of_different_tokens_all_succeed() {
    let h = start().await;
    let mut sessions = Vec::new();
    for n in 1..=8 {
        sessions.push(h.session(n).await);
    }
    let mut set = tokio::task::JoinSet::new();
    for session in &sessions {
        let http = h.http.clone();
        let url = h.url("/oauth/token");
        let refresh = session.refresh().to_owned();
        set.spawn(async move {
            http.post(url)
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("client_secret", CLIENT_SECRET),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.as_str()),
                ])
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        });
    }
    let mut ok = 0;
    while let Some(status) = set.join_next().await {
        assert_eq!(status.unwrap(), 200);
        ok += 1;
    }
    assert_eq!(ok, 8);
}

// spec: B60
#[tokio::test]
async fn sessions_are_independent() {
    let h = start().await;
    let a1 = h.session(1).await;
    let b1 = h.session(2).await;

    let a2 = h.refresh_ok(a1.refresh()).await;
    let a3 = h.refresh_ok(a2.refresh()).await;
    assert_sc_error(h.refresh(a1.refresh()).await, 400, "invalid_grant").await;

    // B is untouched by A's rotations and by A's replay.
    assert!(h.server.token_info(&b1.access_token).is_some());
    let b2 = h.refresh_ok(b1.refresh()).await;
    assert_ne!(b2.refresh(), a3.refresh());
    assert!(h.server.token_info(&a3.access_token).is_some());
    assert!(h.server.token_info(&b2.access_token).is_some());
}

// spec: B60
#[tokio::test]
async fn two_logins_with_the_same_verifier_get_independent_sessions() {
    let h = start().await;
    let v = verifier(1);
    let login_a = h.login_with(&v, "s").await;
    let login_b = h.login_with(&v, "s").await;
    assert_ne!(login_a.code, login_b.code);
    let a = h.exchange_ok(&login_a).await;
    let b = h.exchange_ok(&login_b).await;
    assert_ne!(a.access_token, b.access_token);
    assert_ne!(a.refresh(), b.refresh());
}
