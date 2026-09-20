//! The OAuth 2.1 authorization server: `GET /authorize` and `POST /oauth/token`.

pub(crate) mod authorize;
mod client_auth;
pub(crate) mod store;
pub(crate) mod token;
