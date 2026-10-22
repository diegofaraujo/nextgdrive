use nextgdrive_application::ports::{AccessToken, DriveAccountInfo, DriveService, PortError};
use nextgdrive_domain::{Account, AccountId, Email, StorageQuota};
use serde::Deserialize;

const ABOUT_URL: &str = "https://www.googleapis.com/drive/v3/about";
const ABOUT_FIELDS: &str = "user(displayName,emailAddress,permissionId),storageQuota(limit,usage)";

/// Google Drive API v3 client.
pub struct GoogleDrive {
    http: oauth2::reqwest::Client,
}

impl GoogleDrive {
    pub fn new(http: oauth2::reqwest::Client) -> Self {
        Self { http }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct About {
    user: User,
    storage_quota: Quota,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    display_name: Option<String>,
    email_address: String,
    /// Stable user id, unlike the email.
    permission_id: String,
}

/// The API encodes int64 values as strings.
#[derive(Deserialize)]
struct Quota {
    /// Absent when storage is unlimited.
    limit: Option<String>,
    usage: Option<String>,
}

#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

impl DriveService for GoogleDrive {
    async fn account_info(&self, token: &AccessToken) -> Result<DriveAccountInfo, PortError> {
        let response = self
            .http
            .get(ABOUT_URL)
            .query(&[("fields", ABOUT_FIELDS)])
            .bearer_auth(token.secret())
            .send()
            .await
            .map_err(PortError::new)?;
        let status = response.status();
        let body = response.bytes().await.map_err(PortError::new)?;
        if !status.is_success() {
            let message = serde_json::from_slice::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or_else(|_| String::from_utf8_lossy(&body).into_owned());
            return Err(PortError::new(format!("HTTP {status}: {message}")));
        }
        parse_about(&body)
    }
}

fn parse_about(body: &[u8]) -> Result<DriveAccountInfo, PortError> {
    let about: About = serde_json::from_slice(body).map_err(PortError::new)?;
    let number = |value: Option<String>| {
        value
            .map(|v| v.parse::<u64>().map_err(PortError::new))
            .transpose()
    };
    Ok(DriveAccountInfo {
        account: Account {
            id: AccountId::new(about.user.permission_id),
            email: Email::parse(&about.user.email_address).map_err(PortError::new)?,
            display_name: about.user.display_name.filter(|name| !name.is_empty()),
        },
        quota: StorageQuota {
            usage: number(about.storage_quota.usage)?.unwrap_or(0),
            limit: number(about.storage_quota.limit)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_about_response() {
        let info = parse_about(
            br#"{"user":{"displayName":"Ana Souza","emailAddress":"Ana@Example.com","permissionId":"0123"},
                 "storageQuota":{"limit":"16106127360","usage":"5368709120"}}"#,
        )
        .unwrap();
        assert_eq!(info.account.id.as_str(), "0123");
        assert_eq!(info.account.email.as_str(), "ana@example.com");
        assert_eq!(info.account.display_name.as_deref(), Some("Ana Souza"));
        assert_eq!(
            info.quota,
            StorageQuota {
                usage: 5_368_709_120,
                limit: Some(16_106_127_360)
            }
        );
    }

    #[test]
    fn unlimited_storage_has_no_limit() {
        let info = parse_about(
            br#"{"user":{"emailAddress":"a@x.com","permissionId":"1"},"storageQuota":{"usage":"10"}}"#,
        )
        .unwrap();
        assert_eq!(info.quota.limit, None);
    }
}
