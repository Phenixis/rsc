//! `GET /authorize`: the authorization endpoint (RFC 6749 section 3.1, PKCE mandatory).
//!
//! The real endpoint shows a login page; the mock has no UI and answers according to the
//! configured [`UserDecision`] instead.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, LOCATION};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use url::form_urlencoded;

use crate::config::{RegisteredClient, UserDecision};
use crate::error::ApiError;
use crate::form::Params;
use crate::pkce;
use crate::response::NO_STORE;
use crate::server::Shared;

pub(crate) async fn handler(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    authorize(&shared, &request).unwrap_or_else(ApiError::into_response)
}

fn authorize(shared: &Shared, request: &Request) -> Result<Response, ApiError> {
    // HEAD is GET without a body; hyper strips the body for us.
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return Err(ApiError::method_not_allowed("GET"));
    }
    let params = Params::parse(request.uri().query().unwrap_or_default().as_bytes());

    // Until the redirect URI is trusted, no failure may redirect: RFC 6749 section 3.1.2.4
    // forbids sending the user agent to an unverified URI, and reporting an error there would
    // turn the endpoint into an open redirector.
    let client = known_client(shared, &params)?;
    let redirect = Redirect {
        base: registered_redirect_uri(client, &params)?,
        state: params.first("state"),
    };

    let code_challenge = match validate(&params) {
        Ok(code_challenge) => code_challenge,
        Err(error) => return redirect.to(("error", error)),
    };

    // Only an otherwise valid request reaches the user, as on the real login page.
    let mut state = shared.lock();
    if state.user_decision == UserDecision::Deny {
        return redirect.to(("error", "access_denied"));
    }
    let now = state.clock.now();
    let code = state
        .store
        .issue_code(&client.client_id, redirect.base, code_challenge, now);
    drop(state);

    redirect.to(("code", &code))
}

/// The client named by `client_id`. The parameter must appear exactly once (RFC 6749 section
/// 3.1); the answer is the same `invalid_client` for missing and unknown ids.
fn known_client<'a>(shared: &'a Shared, params: &Params) -> Result<&'a RegisteredClient, ApiError> {
    let client_id = params
        .unique("client_id")
        .map_err(|_| ApiError::invalid_request())?;
    client_id
        .and_then(|client_id| shared.client(client_id))
        .ok_or_else(|| ApiError::oauth(StatusCode::BAD_REQUEST, "invalid_client"))
}

/// The registered URI that `redirect_uri` equals byte for byte. RFC 6749 section 3.1.2.3
/// requires a comparison of the whole string; the mock accepts no near miss (another host
/// spelling, a trailing slash, an added query...).
fn registered_redirect_uri<'a>(
    client: &'a RegisteredClient,
    params: &Params,
) -> Result<&'a str, ApiError> {
    let requested = params
        .unique("redirect_uri")
        .map_err(|_| ApiError::invalid_request())?
        .ok_or_else(ApiError::invalid_request)?;
    client
        .redirect_uris
        .iter()
        .map(String::as_str)
        .find(|registered| *registered == requested)
        .ok_or_else(ApiError::invalid_request)
}

/// Checks the parameters that are reported by redirect once the redirect URI is trusted, in
/// the documented order, and returns the code challenge. The error is the `error=` code.
fn validate(params: &Params) -> Result<&str, &'static str> {
    const INVALID_REQUEST: &str = "invalid_request";

    match params
        .unique("response_type")
        .map_err(|_| INVALID_REQUEST)?
    {
        Some("code") => {}
        None => return Err(INVALID_REQUEST),
        Some(_) => return Err("unsupported_response_type"),
    }

    // PKCE is mandatory (OAuth 2.1), and only S256: `plain` protects nothing, and a missing
    // method must not silently mean `plain` (RFC 7636 section 4.3 default).
    let code_challenge = params
        .unique("code_challenge")
        .map_err(|_| INVALID_REQUEST)?
        .filter(|challenge| pkce::is_valid_challenge(challenge))
        .ok_or(INVALID_REQUEST)?;
    match params
        .unique("code_challenge_method")
        .map_err(|_| INVALID_REQUEST)?
    {
        Some("S256") => {}
        _ => return Err(INVALID_REQUEST),
    }

    // A repeated `state` would make it ambiguous what to echo.
    params.unique("state").map_err(|_| INVALID_REQUEST)?;

    Ok(code_challenge)
}

/// A redirect back to the client, carrying the `state` of the request when it had one.
struct Redirect<'a> {
    /// The registered redirect URI, used verbatim.
    base: &'a str,
    state: Option<&'a str>,
}

impl Redirect<'_> {
    /// `302` to the redirect URI with the given parameter, then `state`, appended to whatever
    /// query the registered URI already has.
    fn to(&self, (name, value): (&str, &str)) -> Result<Response, ApiError> {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair(name, value);
        if let Some(state) = self.state {
            query.append_pair("state", state);
        }
        let query = query.finish();

        let separator = if !self.base.contains('?') {
            "?"
        } else if self.base.ends_with(['?', '&']) {
            ""
        } else {
            "&"
        };
        let location = HeaderValue::try_from(format!("{}{separator}{query}", self.base))
            .map_err(|_| ApiError::internal())?;

        Ok((
            StatusCode::FOUND,
            [(LOCATION, location), (CACHE_CONTROL, NO_STORE)],
        )
            .into_response())
    }
}
