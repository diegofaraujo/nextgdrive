use std::path::{Path, PathBuf};
use std::{env, fs, io};

use serde::Deserialize;
use thiserror::Error;

pub const CLIENT_ID_VAR: &str = "NEXTGDRIVE_CLIENT_ID";
pub const CLIENT_SECRET_VAR: &str = "NEXTGDRIVE_CLIENT_SECRET";

/// OAuth client registered in Google Cloud ("Desktop app" type).
///
/// For installed apps Google does not treat the secret as confidential, but it
/// is still kept out of the repository: each developer uses their own client.
/// Release builds embed the project's client, read from the same variables at
/// build time.
#[derive(Clone)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Error)]
pub enum ClientConfigError {
    #[error(
        "OAuth client not configured: set {CLIENT_ID_VAR} and {CLIENT_SECRET_VAR}, or create {0}"
    )]
    NotFound(PathBuf),
    #[error("could not read OAuth client file {0}: {1}")]
    Unreadable(PathBuf, String),
}

/// File downloaded from Google Cloud Console.
#[derive(Deserialize)]
struct ClientFile {
    installed: ClientEntry,
}

#[derive(Deserialize)]
struct ClientEntry {
    client_id: String,
    client_secret: String,
}

impl ClientConfig {
    /// Reads the environment variables if both are set, otherwise the JSON file
    /// downloaded from Google Cloud Console, otherwise the client embedded in
    /// the build (if any). Your own client always wins over the project's.
    pub fn load(file: &Path) -> Result<Self, ClientConfigError> {
        Self::load_or(file, Self::embedded())
    }

    /// The client set in the environment when this was built.
    pub fn embedded() -> Option<Self> {
        match (
            option_env!("NEXTGDRIVE_CLIENT_ID"),
            option_env!("NEXTGDRIVE_CLIENT_SECRET"),
        ) {
            (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Some(Self {
                client_id: id.to_owned(),
                client_secret: secret.to_owned(),
            }),
            _ => None,
        }
    }

    fn load_or(file: &Path, embedded: Option<Self>) -> Result<Self, ClientConfigError> {
        if let (Ok(client_id), Ok(client_secret)) =
            (env::var(CLIENT_ID_VAR), env::var(CLIENT_SECRET_VAR))
        {
            return Ok(Self {
                client_id,
                client_secret,
            });
        }
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return embedded.ok_or_else(|| ClientConfigError::NotFound(file.to_owned()));
            }
            Err(e) => {
                return Err(ClientConfigError::Unreadable(
                    file.to_owned(),
                    e.to_string(),
                ));
            }
        };
        Self::parse(&text)
            .map_err(|e| ClientConfigError::Unreadable(file.to_owned(), e.to_string()))
    }

    fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let ClientFile { installed } = serde_json::from_str(json)?;
        Ok(Self {
            client_id: installed.client_id,
            client_secret: installed.client_secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_google_desktop_client_file() {
        let config = ClientConfig::parse(
            r#"{"installed":{"client_id":"id.apps.googleusercontent.com","project_id":"p",
                "client_secret":"s","redirect_uris":["http://localhost"]}}"#,
        )
        .unwrap();
        assert_eq!(config.client_id, "id.apps.googleusercontent.com");
        assert_eq!(config.client_secret, "s");
    }

    #[test]
    fn your_own_client_file_wins_over_the_embedded_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("oauth-client.json");
        let embedded = || {
            Some(ClientConfig {
                client_id: "project".into(),
                client_secret: "s".into(),
            })
        };
        // Only meaningful when the variables are not set in the test's environment.
        if env::var(CLIENT_ID_VAR).is_ok() {
            return;
        }

        assert_eq!(
            ClientConfig::load_or(&file, embedded()).unwrap().client_id,
            "project"
        );
        assert!(matches!(
            ClientConfig::load_or(&file, None),
            Err(ClientConfigError::NotFound(_))
        ));
        fs::write(
            &file,
            r#"{"installed":{"client_id":"mine","client_secret":"s"}}"#,
        )
        .unwrap();
        assert_eq!(
            ClientConfig::load_or(&file, embedded()).unwrap().client_id,
            "mine"
        );
    }

    #[test]
    fn rejects_web_client_file() {
        assert!(ClientConfig::parse(r#"{"web":{"client_id":"id","client_secret":"s"}}"#).is_err());
    }
}
