//! PKCE (RFC 7636): code verifier / S256 challenge, plus the anti-CSRF `state`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// `base64url_nopad(sha256(verifier))`.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 32 random bytes from the OS-seeded CSPRNG, base64url without padding (43 characters).
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Fresh random verifier (32 random bytes -> 43 characters) and its challenge.
pub fn generate() -> Pkce {
    let verifier = random_token();
    Pkce {
        challenge: challenge_for(&verifier),
        verifier,
    }
}

/// Fresh random `state` value.
pub fn random_state() -> String {
    random_token()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_base64url(s: &str) -> bool {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// RFC 7636, Appendix B.
    #[test]
    fn challenge_matches_the_rfc_test_vector() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifier_is_43_unpadded_base64url_characters() {
        let pkce = generate();
        assert_eq!(pkce.verifier.len(), 43);
        assert!(is_base64url(&pkce.verifier), "{}", pkce.verifier);
    }

    #[test]
    fn generated_challenge_belongs_to_the_verifier() {
        let pkce = generate();
        assert_eq!(pkce.challenge, challenge_for(&pkce.verifier));
        assert_eq!(pkce.challenge.len(), 43); // sha256 = 32 bytes
    }

    #[test]
    fn generated_values_are_not_reused() {
        assert_ne!(generate().verifier, generate().verifier);
        assert_ne!(random_state(), random_state());
    }

    #[test]
    fn state_is_url_safe_and_long_enough_to_be_unguessable() {
        let state = random_state();
        assert!(state.len() >= 22, "{state}"); // >= 128 bits
        assert!(is_base64url(&state), "{state}");
    }
}
