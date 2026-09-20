//! `POST /oauth/token`: the token endpoint (RFC 6749 section 3.2) for the `authorization_code`
//! (with PKCE), `refresh_token` and `client_credentials` grants.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, PRAGMA};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::Serialize;

use super::client_auth::{self, Authenticated, Method as AuthMethod};
use super::store::{CodeRedemption, InvalidGrant, Issued};
use crate::error::ApiError;
use crate::form::Params;
use crate::response;
use crate::server::Shared;

/// Largest request body read: a token request is a handful of short parameters.
const MAX_BODY_BYTES: usize = 64 * 1024;

const FORM_MEDIA_TYPE: &str = "application/x-www-form-urlencoded";

/// The parameters this endpoint understands. Unknown ones are ignored (RFC 6749 section 3.2),
/// but a known one sent twice is an error.
const KNOWN_PARAMETERS: [&str; 7] = [
    "grant_type",
    "client_id",
    "client_secret",
    "code",
    "redirect_uri",
    "code_verifier",
    "refresh_token",
];

/// The JSON of a successful token response. `refresh_token` is absent (not `null`) for
/// `client_credentials`.
#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    expires_in: u64,
    scope: &'static str,
    token_type: &'static str,
}

impl From<InvalidGrant> for ApiError {
    fn from(_: InvalidGrant) -> Self {
        ApiError::invalid_grant()
    }
}

pub(crate) async fn handler(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    token_endpoint(&shared, request)
        .await
        .unwrap_or_else(ApiError::into_response)
}

async fn token_endpoint(shared: &Shared, request: Request) -> Result<Response, ApiError> {
    if request.method() != Method::POST {
        return Err(ApiError::method_not_allowed("POST"));
    }
    require_form_content_type(request.headers())?;

    let (parts, body) = request.into_parts();
    let body = read_body(&parts.headers, body).await?;
    // Only the body counts: credentials or grant parameters in the URL query are ignored, so
    // they cannot end up in logs and proxies by accident.
    let params = Params::parse(&body);

    // Before anything is looked at, let alone consumed: which of two copies would count?
    for name in KNOWN_PARAMETERS {
        params
            .unique(name)
            .map_err(|_| ApiError::invalid_request())?;
    }

    // Client authentication comes before the grant: an unauthenticated caller learns nothing
    // about which grants exist or what a code looks like.
    let client = client_auth::authenticate(&shared.config.clients, &parts.headers, &params)?;

    match params.value("grant_type") {
        None => Err(ApiError::invalid_request()),
        Some("authorization_code") => authorization_code(shared, &client, &params),
        Some("refresh_token") => refresh_token(shared, &client, &params),
        Some("client_credentials") => client_credentials(shared, &client),
        Some(_) => Err(ApiError::oauth(
            StatusCode::BAD_REQUEST,
            "unsupported_grant",
        )),
    }
}

fn authorization_code(
    shared: &Shared,
    client: &Authenticated<'_>,
    params: &Params,
) -> Result<Response, ApiError> {
    let (Some(code), Some(redirect_uri), Some(code_verifier)) = (
        params.value("code"),
        params.value("redirect_uri"),
        params.value("code_verifier"),
    ) else {
        return Err(ApiError::invalid_request());
    };

    let request = CodeRedemption {
        client_id: &client.client.client_id,
        code,
        redirect_uri,
        code_verifier,
    };
    let mut state = shared.lock();
    let now = state.clock.now();
    let issued = state.store.redeem_code(&request, now)?;
    drop(state);

    Ok(token_response(issued, shared.config.access_token_lifetime))
}

fn refresh_token(
    shared: &Shared,
    client: &Authenticated<'_>,
    params: &Params,
) -> Result<Response, ApiError> {
    // `redirect_uri` and `scope` are ignored: clients rarely send them on refresh, and the
    // mock has no scopes.
    let refresh_token = params
        .value("refresh_token")
        .ok_or_else(ApiError::invalid_request)?;

    let mut state = shared.lock();
    let now = state.clock.now();
    let issued = state
        .store
        .refresh(&client.client.client_id, refresh_token, now)?;
    drop(state);

    Ok(token_response(issued, shared.config.access_token_lifetime))
}

fn client_credentials(shared: &Shared, client: &Authenticated<'_>) -> Result<Response, ApiError> {
    // SoundCloud's guide only documents HTTP Basic for this grant.
    if client.method != AuthMethod::Basic {
        return Err(ApiError::invalid_client_credentials());
    }

    let mut state = shared.lock();
    let now = state.clock.now();
    let issued = state.store.issue_app_token(&client.client.client_id, now);
    drop(state);

    Ok(token_response(issued, shared.config.access_token_lifetime))
}

fn token_response(issued: Issued, access_token_lifetime: Duration) -> Response {
    let body = TokenResponse {
        access_token: issued.access_token,
        refresh_token: issued.refresh_token,
        expires_in: access_token_lifetime.as_secs(),
        scope: "",
        token_type: "bearer",
    };
    let mut response = response::json(StatusCode::OK, &body);
    // RFC 6749 section 5.1: token responses must not be cached.
    response
        .headers_mut()
        .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// The media type must be `application/x-www-form-urlencoded` (RFC 6749 section 3.2), compared
/// case-insensitively and ignoring parameters such as `charset`.
fn require_form_content_type(headers: &HeaderMap) -> Result<(), ApiError> {
    let is_form = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split_once(';')
                .map_or(value, |(media_type, _)| media_type)
        })
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(FORM_MEDIA_TYPE));
    if is_form {
        Ok(())
    } else {
        Err(ApiError::oauth(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ))
    }
}

/// Reads the body, refusing more than [`MAX_BODY_BYTES`].
async fn read_body(headers: &HeaderMap, body: Body) -> Result<Vec<u8>, ApiError> {
    let payload_too_large = || ApiError::oauth(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");

    // Refuse an oversized body from its announced length, without reading any of it.
    let announced = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if announced.is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return Err(payload_too_large());
    }

    // The limit must also hold for chunked bodies, which announce no length.
    match Limited::new(body, MAX_BODY_BYTES).collect().await {
        Ok(collected) => Ok(collected.to_bytes().to_vec()),
        Err(error) if error.is::<LengthLimitError>() => Err(payload_too_large()),
        // The client went away or sent a malformed body.
        Err(_) => Err(ApiError::invalid_request()),
    }
}
