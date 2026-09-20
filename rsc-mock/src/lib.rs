//! `rsc-mock`: a faithful fake of the SoundCloud API, so that `rsc` can be developed and tested
//! end to end without an official app.
//!
//! A single mock server answers both the authorization endpoints (`/authorize`,
//! `/oauth/token`) and, in later slices, the API endpoints: point a client's auth and API bases
//! at [`MockServer::base_url`]. Time is a manual clock ([`MockServer::advance_clock`]), so tests
//! never sleep.
//!
//! ```no_run
//! # async fn demo() -> std::io::Result<()> {
//! let server = rsc_mock::spawn(rsc_mock::MockConfig::default()).await?;
//! println!("mock listening on {}", server.base_url());
//! server.shutdown().await;
//! # Ok(())
//! # }
//! ```

mod clock;
mod config;
mod error;
mod form;
mod oauth;
pub mod pkce;
mod response;
mod server;

pub use config::{MockConfig, RegisteredClient, UserDecision};
pub use server::{MockServer, TokenInfo, TokenKind, spawn};
