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
mod tests;
