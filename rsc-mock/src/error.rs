//! SoundCloud's error body: `{"code": <status>, "message": <m>, "link": <l>}`, not RFC 6749's
//! `{"error": ...}`.

use axum::http::header::{ALLOW, WWW_AUTHENTICATE};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::response;

/// Link of the errors of the authorization server.
const OAUTH_LINK: &str = "https://developers.soundcloud.com/docs/api/guide#authentication";
/// Link of the errors of the API endpoints.
const API_LINK: &str = "https://developers.soundcloud.com/docs/api/explorer/open-api";

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    message: &'static str,
    link: &'static str,
    header: Option<(HeaderName, HeaderValue)>,
}

#[derive(Serialize)]
struct Body {
    code: u16,
    message: &'static str,
    link: &'static str,
}

impl ApiError {
    /// An error of the authorization server: `message` is the OAuth error code.
    pub(crate) fn oauth(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            message,
            link: OAUTH_LINK,
            header: None,
        }
    }

    pub(crate) fn invalid_request() -> Self {
        Self::oauth(StatusCode::BAD_REQUEST, "invalid_request")
    }

    pub(crate) fn invalid_grant() -> Self {
        Self::oauth(StatusCode::BAD_REQUEST, "invalid_grant")
    }

    /// Failed client authentication. RFC 6749 section 5.2: a `401` must say which scheme the
    /// client can use to authenticate.
    pub(crate) fn invalid_client_credentials() -> Self {
        Self {
            header: Some((
                WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"oauth\""),
            )),
            ..Self::oauth(StatusCode::UNAUTHORIZED, "invalid_client")
        }
    }

    pub(crate) fn method_not_allowed(allowed: &'static str) -> Self {
        Self {
            header: Some((ALLOW, HeaderValue::from_static(allowed))),
            ..Self::oauth(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
        }
    }

    pub(crate) fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "not_found",
            link: API_LINK,
            header: None,
        }
    }

    pub(crate) fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal_error",
            link: API_LINK,
            header: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Body {
            code: self.status.as_u16(),
            message: self.message,
            link: self.link,
        };
        let mut response = response::json(self.status, &body);
        if let Some((name, value)) = self.header {
            response.headers_mut().insert(name, value);
        }
        response
    }
}
