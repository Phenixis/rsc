//! Helpers shared by every endpoint to build responses.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Credentials, codes and errors must never be stored by a cache.
pub(crate) const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

const JSON_UTF8: HeaderValue = HeaderValue::from_static("application/json; charset=utf-8");

/// A JSON response with SoundCloud's content type and `Cache-Control: no-store`.
pub(crate) fn json<T: Serialize>(status: StatusCode, body: &T) -> Response {
    match serde_json::to_vec(body) {
        Ok(bytes) => (
            status,
            [(CONTENT_TYPE, JSON_UTF8), (CACHE_CONTROL, NO_STORE)],
            bytes,
        )
            .into_response(),
        // Unreachable for the plain data structures we serialize, but never panic on it.
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
