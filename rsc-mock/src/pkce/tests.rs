use super::{is_valid_challenge, is_valid_verifier, s256_challenge, verify_s256};

/// RFC 7636, appendix B.
const RFC_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const RFC_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

fn repeat(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

// spec: B58
#[test]
fn s256_challenge_matches_the_rfc_7636_appendix_b_vector() {
    assert_eq!(s256_challenge(RFC_VERIFIER), RFC_CHALLENGE);
}

// spec: B58
#[test]
fn s256_challenge_has_43_unpadded_url_safe_characters_for_any_verifier() {
    for verifier in [
        repeat('a', 43),
        repeat('Z', 64),
        repeat('~', 128),
        format!("{}-._~", repeat('0', 40)),
    ] {
        let challenge = s256_challenge(&verifier);
        assert_eq!(challenge.len(), 43, "challenge of {verifier}");
        assert!(
            challenge
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "challenge {challenge} must be base64url without padding"
        );
    }
}

// spec: B58
#[test]
fn s256_challenge_differs_between_verifiers() {
    assert_ne!(
        s256_challenge(&repeat('a', 43)),
        s256_challenge(&repeat('b', 43))
    );
    assert_ne!(
        s256_challenge(&repeat('a', 43)),
        s256_challenge(&repeat('a', 44))
    );
}

// spec: B58
#[test]
fn verify_s256_accepts_the_rfc_vector() {
    assert!(verify_s256(RFC_VERIFIER, RFC_CHALLENGE));
}

// spec: B58
#[test]
fn verify_s256_accepts_any_valid_verifier_with_its_own_challenge() {
    for verifier in [repeat('x', 43), repeat('Q', 100), repeat('7', 128)] {
        assert!(verify_s256(&verifier, &s256_challenge(&verifier)));
    }
}

// spec: B58
#[test]
fn verify_s256_rejects_a_different_verifier_or_challenge() {
    // Last character changed.
    assert!(RFC_VERIFIER.ends_with('k'));
    let almost = format!("{}j", &RFC_VERIFIER[..RFC_VERIFIER.len() - 1]);
    assert!(!verify_s256(&almost, RFC_CHALLENGE));
    // The challenge itself used as verifier (a classic "plain" mix-up).
    assert!(!verify_s256(RFC_CHALLENGE, RFC_CHALLENGE));
    // Right verifier, challenge of another verifier.
    assert!(!verify_s256(
        RFC_VERIFIER,
        &s256_challenge(&repeat('a', 43))
    ));
    // Empty inputs.
    assert!(!verify_s256("", ""));
    assert!(!verify_s256(RFC_VERIFIER, ""));
}

// spec: B58
#[test]
fn verify_s256_rejects_a_verifier_that_is_not_well_formed_even_if_the_hash_matches() {
    for bad in [
        repeat('a', 42),
        repeat('a', 129),
        format!("{} ", repeat('a', 42)),
    ] {
        let challenge = s256_challenge(&bad);
        assert!(!verify_s256(&bad, &challenge), "verifier {bad:?}");
    }
}

// spec: B58
#[test]
fn verifier_length_bounds_are_43_to_128_inclusive() {
    assert!(!is_valid_verifier(""));
    assert!(!is_valid_verifier(&repeat('a', 42)));
    assert!(is_valid_verifier(&repeat('a', 43)));
    assert!(is_valid_verifier(&repeat('a', 44)));
    assert!(is_valid_verifier(&repeat('a', 127)));
    assert!(is_valid_verifier(&repeat('a', 128)));
    assert!(!is_valid_verifier(&repeat('a', 129)));
    assert!(!is_valid_verifier(&repeat('a', 5000)));
}

// spec: B58
#[test]
fn verifier_accepts_exactly_the_unreserved_characters() {
    // ALPHA / DIGIT / "-" / "." / "_" / "~"
    let all = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    assert_eq!(all.chars().count(), 66);
    // The whole alphabet once: 66 characters, within the 43..=128 limit.
    assert!(is_valid_verifier(all));
    // Every single unreserved character is accepted on its own (in the last position, after
    // 42 others, so that the length is exactly the minimum).
    for good in all.chars() {
        let verifier = format!("{}{}", repeat('a', 42), good);
        assert!(is_valid_verifier(&verifier), "{good:?} must be accepted");
        let verifier = format!("{good}{}", repeat('a', 42));
        assert!(
            is_valid_verifier(&verifier),
            "{good:?} must be accepted first"
        );
    }
    // A 128-character verifier that cycles through the alphabet (still within the limit).
    let cycled: String = all.chars().cycle().take(128).collect();
    assert_eq!(cycled.chars().count(), 128);
    assert!(is_valid_verifier(&cycled));
    // One more character and it is too long, whatever the character.
    assert!(!is_valid_verifier(&format!("{cycled}a")));
    for bad in [
        ' ',
        '+',
        '/',
        '=',
        '%',
        '!',
        '@',
        ':',
        '\n',
        '\u{e9}',
        '\u{1f600}',
    ] {
        let verifier = format!("{}{}", repeat('a', 42), bad);
        // Same number of characters or more: only the alphabet can be at fault.
        assert!(!is_valid_verifier(&verifier), "{bad:?} must be rejected");
    }
    // 43 multi-byte characters are not valid either (length in bytes would be 86).
    assert!(!is_valid_verifier(&repeat('\u{e9}', 43)));
}

// spec: B58
#[test]
fn challenge_must_be_43_url_safe_characters() {
    assert!(is_valid_challenge(RFC_CHALLENGE));
    assert!(is_valid_challenge(&repeat('A', 43)));
    assert!(is_valid_challenge(&format!("{}-_", repeat('9', 41))));
    assert!(!is_valid_challenge(""));
    assert!(!is_valid_challenge(&repeat('A', 42)));
    assert!(!is_valid_challenge(&repeat('A', 44)));
    // Padded base64 of a 32-byte digest is 44 characters ending in '='.
    assert!(!is_valid_challenge(&format!("{RFC_CHALLENGE}=")));
    // Standard (non URL-safe) alphabet.
    assert!(!is_valid_challenge(&format!("{}+", repeat('A', 42))));
    assert!(!is_valid_challenge(&format!("{}/", repeat('A', 42))));
    assert!(!is_valid_challenge(&format!("{} ", repeat('A', 42))));
    assert!(!is_valid_challenge(&repeat('\u{e9}', 43)));
}
