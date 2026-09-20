//! PKCE (RFC 7636), S256 method only: `plain` is never accepted by the mock.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Length of an unpadded base64url SHA-256 digest.
const CHALLENGE_LEN: usize = 43;
/// RFC 7636 section 4.1 bounds of a code verifier.
const VERIFIER_LEN: std::ops::RangeInclusive<usize> = 43..=128;

/// `base64url_nopad(sha256(verifier))` (RFC 7636 section 4.2). Does not validate the input.
pub fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// RFC 7636 section 4.1: 43 to 128 characters, all from ALPHA / DIGIT / "-" / "." / "_" / "~".
/// Length is counted in characters; any non-ASCII character makes the verifier invalid.
pub fn is_valid_verifier(verifier: &str) -> bool {
    // Every accepted byte is ASCII, so the byte length equals the character length; a
    // multi-byte character is rejected by the alphabet check whatever the length says.
    VERIFIER_LEN.contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// An S256 challenge is the unpadded base64url of a 32-byte digest: exactly 43 characters
/// from ALPHA / DIGIT / "-" / "_". Padding ("=") and the standard alphabet ("+", "/") are invalid.
pub fn is_valid_challenge(challenge: &str) -> bool {
    challenge.len() == CHALLENGE_LEN
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// `is_valid_verifier(verifier) && s256_challenge(verifier) == challenge`.
/// The comparison of the two digests is constant-time.
pub fn verify_s256(verifier: &str, challenge: &str) -> bool {
    is_valid_verifier(verifier)
        && bool::from(
            s256_challenge(verifier)
                .as_bytes()
                .ct_eq(challenge.as_bytes()),
        )
}

#[cfg(test)]
mod tests;
