//! Client authentication at the token endpoint (RFC 6749 section 2.3).

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::config::RegisteredClient;
use crate::error::ApiError;
use crate::form::Params;

/// How the client presented its credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    /// `Authorization: Basic base64(client_id:client_secret)`.
    Basic,
    /// `client_id` and `client_secret` in the form body.
    Body,
}

#[derive(Debug)]
pub(super) struct Authenticated<'a> {
    pub(super) client: &'a RegisteredClient,
    pub(super) method: Method,
}

/// Standard base64 where clients may or may not have kept the padding.
const BASIC_BASE64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Authenticates the client of a token request.
///
/// A present `Authorization` header is *the* authentication attempt: if it is unusable the
/// request fails even when the body carries valid credentials, instead of silently falling back
/// to them. All failures look the same (`401`), so nothing tells an unknown client from a wrong
/// secret.
pub(super) fn authenticate<'a>(
    clients: &'a [RegisteredClient],
    headers: &HeaderMap,
    params: &Params,
) -> Result<Authenticated<'a>, ApiError> {
    let (method, client_id, client_secret) = if headers.contains_key(AUTHORIZATION) {
        let (client_id, client_secret) =
            basic_credentials(headers).ok_or_else(ApiError::invalid_client_credentials)?;

        // RFC 6749 section 2.3: a client MUST NOT use more than one authentication method per
        // request.
        if params.contains("client_secret") {
            return Err(ApiError::invalid_request());
        }
        // An id in the body may only repeat the header's.
        if params
            .value("client_id")
            .is_some_and(|body_id| body_id != client_id)
        {
            return Err(ApiError::invalid_client_credentials());
        }
        (Method::Basic, client_id, client_secret)
    } else {
        match (params.value("client_id"), params.value("client_secret")) {
            (Some(client_id), Some(client_secret)) => {
                (Method::Body, client_id.to_owned(), client_secret.to_owned())
            }
            _ => return Err(ApiError::invalid_client_credentials()),
        }
    };

    let client = clients.iter().find(|client| client.client_id == client_id);
    // The secret is compared even for an unknown client so that both failures cost the same.
    let expected = client.map_or("", |client| client.client_secret.as_str());
    let secret_matches = secrets_equal(expected, &client_secret);

    match client {
        Some(client) if secret_matches => Ok(Authenticated { client, method }),
        _ => Err(ApiError::invalid_client_credentials()),
    }
}

/// The `(client_id, client_secret)` of a well-formed Basic header, `None` for anything else
/// (another scheme, bad base64, no colon, an empty id or secret, several headers).
///
/// The decoded value is split at the first colon, so secrets may contain colons; the parts are
/// used raw, not percent-decoded.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }

    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = BASIC_BASE64.decode(encoded.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (client_id, client_secret) = decoded.split_once(':')?;
    if client_id.is_empty() || client_secret.is_empty() {
        return None;
    }
    Some((client_id.to_owned(), client_secret.to_owned()))
}

/// Constant-time comparison. Hashing first makes both sides the same length, so even the
/// length of the real secret does not leak.
fn secrets_equal(expected: &str, presented: &str) -> bool {
    let expected = Sha256::digest(expected.as_bytes());
    let presented = Sha256::digest(presented.as_bytes());
    bool::from(expected.ct_eq(&presented))
}
