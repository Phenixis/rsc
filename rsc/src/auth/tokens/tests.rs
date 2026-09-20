use super::*;

fn response(expires_in: u64) -> TokenResponse {
    TokenResponse {
        access_token: "access-secret".into(),
        refresh_token: "refresh-secret".into(),
        expires_in,
        scope: String::new(),
    }
}

#[test]
fn expiry_is_now_plus_lifetime_minus_the_safety_margin() {
    let tokens = Tokens::from_response(response(3600), 1000);
    assert_eq!(tokens.expires_at, 1000 + 3600 - EXPIRY_MARGIN_SECS);
    assert_eq!(tokens.access_token, "access-secret");
    assert_eq!(tokens.refresh_token, "refresh-secret");
}

#[test]
fn a_lifetime_shorter_than_the_margin_is_already_expired_not_an_underflow() {
    let tokens = Tokens::from_response(response(30), 1000);
    assert!(tokens.is_expired(1000));
}

#[test]
fn is_expired_flips_exactly_at_expires_at() {
    let tokens = Tokens {
        access_token: "a".into(),
        refresh_token: "r".into(),
        expires_at: 100,
    };
    assert!(!tokens.is_expired(99));
    assert!(tokens.is_expired(100));
    assert!(tokens.is_expired(101));
}

#[test]
fn token_endpoint_json_is_parsed_and_scope_is_optional() {
    let full = r#"{"access_token":"a","refresh_token":"r","expires_in":3599,
                    "scope":"","token_type":"bearer"}"#;
    let parsed: TokenResponse = serde_json::from_str(full).unwrap();
    assert_eq!(
        (parsed.access_token.as_str(), parsed.expires_in),
        ("a", 3599)
    );

    let no_scope = r#"{"access_token":"a","refresh_token":"r","expires_in":1}"#;
    assert!(serde_json::from_str::<TokenResponse>(no_scope).is_ok());
    assert!(serde_json::from_str::<TokenResponse>(r#"{"access_token":"a"}"#).is_err());
}

#[test]
fn debug_output_never_contains_the_secrets() {
    let tokens = Tokens::from_response(response(3600), 0);
    for shown in [format!("{tokens:?}"), format!("{:?}", response(1))] {
        assert!(!shown.contains("access-secret"), "{shown}");
        assert!(!shown.contains("refresh-secret"), "{shown}");
    }
}
