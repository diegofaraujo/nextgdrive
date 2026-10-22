//! Google adapters: OAuth 2.0 sign-in and the Google Drive API.

mod auth;
mod client_config;
mod drive;

pub use auth::{CallbackPage, GoogleAuthenticator, GooglePendingAuthorization};
pub use client_config::{CLIENT_ID_VAR, CLIENT_SECRET_VAR, ClientConfig, ClientConfigError};
pub use drive::GoogleDrive;

/// HTTP client shared by the adapters. Redirects are disabled, as required
/// by OAuth token requests.
pub fn http_client() -> oauth2::reqwest::Client {
    oauth2::reqwest::Client::builder()
        .redirect(oauth2::reqwest::redirect::Policy::none())
        .build()
        .expect("TLS backend is available")
}
