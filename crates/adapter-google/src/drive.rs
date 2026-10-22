use std::time::SystemTime;

use nextgdrive_application::ports::{AccessToken, DriveAccountInfo, DriveService, PortError};
use nextgdrive_domain::{
    Account, AccountId, Drive, DriveItem, DriveItemId, DriveKind, Email, ItemKind, StorageQuota,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;

const API: &str = "https://www.googleapis.com/drive/v3";
const ABOUT_FIELDS: &str = "user(displayName,emailAddress,permissionId),storageQuota(limit,usage)";
const FILE_FIELDS: &str =
    "id,name,mimeType,size,md5Checksum,modifiedTime,parents,webViewLink,capabilities/canDownload";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const SHORTCUT_MIME: &str = "application/vnd.google-apps.shortcut";
/// Docs, Sheets, Slides, Forms, Drawings…: no downloadable content.
const GOOGLE_APPS_MIME_PREFIX: &str = "application/vnd.google-apps.";

/// Google Drive API v3 client.
pub struct GoogleDrive {
    http: oauth2::reqwest::Client,
}

impl GoogleDrive {
    pub fn new(http: oauth2::reqwest::Client) -> Self {
        Self { http }
    }

    async fn get<T: DeserializeOwned>(
        &self,
        token: &AccessToken,
        url: Url,
        query: &[(&str, &str)],
    ) -> Result<T, PortError> {
        let response = self
            .http
            .get(url)
            .query(query)
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
        serde_json::from_slice(&body).map_err(PortError::new)
    }

    /// Follows `nextPageToken` until every page of a list request is fetched.
    async fn get_all<T, P>(
        &self,
        token: &AccessToken,
        url: Url,
        query: &[(&str, &str)],
        mut items: impl FnMut(P) -> (Vec<T>, Option<String>),
    ) -> Result<Vec<T>, PortError>
    where
        P: DeserializeOwned,
    {
        let mut all = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut page_query = query.to_vec();
            if let Some(page) = &page_token {
                page_query.push(("pageToken", page));
            }
            let (batch, next) = items(self.get(token, url.clone(), &page_query).await?);
            all.extend(batch);
            match next {
                Some(next) => page_token = Some(next),
                None => return Ok(all),
            }
        }
    }
}

fn endpoint(segments: &[&str]) -> Url {
    let mut url = Url::parse(API).expect("valid URL");
    // Path segments are percent-encoded, so ids cannot alter the path.
    url.path_segments_mut().expect("base URL").extend(segments);
    url
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
#[serde(rename_all = "camelCase")]
struct File {
    id: String,
    name: String,
    mime_type: String,
    size: Option<String>,
    md5_checksum: Option<String>,
    modified_time: Option<String>,
    #[serde(default)]
    parents: Vec<String>,
    web_view_link: Option<String>,
    capabilities: Option<Capabilities>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    can_download: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    next_page_token: Option<String>,
    #[serde(default)]
    files: Vec<File>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveList {
    next_page_token: Option<String>,
    #[serde(default)]
    drives: Vec<SharedDrive>,
}

#[derive(Deserialize)]
struct SharedDrive {
    id: String,
    name: String,
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
        let about: About = self
            .get(token, endpoint(&["about"]), &[("fields", ABOUT_FIELDS)])
            .await?;
        about_to_info(about)
    }

    async fn drives(&self, token: &AccessToken) -> Result<Vec<Drive>, PortError> {
        let root: File = self
            .get(
                token,
                endpoint(&["files", "root"]),
                &[("fields", FILE_FIELDS)],
            )
            .await?;
        let mut drives = vec![Drive {
            root: DriveItemId::new(root.id),
            kind: DriveKind::MyDrive,
        }];
        let shared = self
            .get_all(
                token,
                endpoint(&["drives"]),
                &[
                    ("pageSize", "100"),
                    ("fields", "nextPageToken,drives(id,name)"),
                ],
                |page: DriveList| (page.drives, page.next_page_token),
            )
            .await?;
        drives.extend(shared.into_iter().map(|d| Drive {
            root: DriveItemId::new(d.id),
            kind: DriveKind::Shared { name: d.name },
        }));
        Ok(drives)
    }

    async fn children(
        &self,
        token: &AccessToken,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError> {
        let q = format!(
            "'{}' in parents and trashed = false",
            escape_query(folder.as_str())
        );
        let fields = format!("nextPageToken,files({FILE_FIELDS})");
        let files = self
            .get_all(
                token,
                endpoint(&["files"]),
                &[
                    ("q", &q),
                    ("fields", &fields),
                    ("pageSize", "1000"),
                    ("corpora", "allDrives"),
                    ("includeItemsFromAllDrives", "true"),
                    ("supportsAllDrives", "true"),
                ],
                |page: FileList| (page.files, page.next_page_token),
            )
            .await?;
        files.into_iter().map(file_to_item).collect()
    }

    async fn item(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        let file: File = self
            .get(
                token,
                endpoint(&["files", id.as_str()]),
                &[("fields", FILE_FIELDS), ("supportsAllDrives", "true")],
            )
            .await?;
        file_to_item(file)
    }
}

/// Escapes a value inside single quotes in a Drive search query.
fn escape_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn about_to_info(about: About) -> Result<DriveAccountInfo, PortError> {
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

fn number(value: Option<String>) -> Result<Option<u64>, PortError> {
    value
        .map(|v| v.parse::<u64>().map_err(PortError::new))
        .transpose()
}

fn file_to_item(file: File) -> Result<DriveItem, PortError> {
    let can_download = file
        .capabilities
        .and_then(|c| c.can_download)
        .unwrap_or(true);
    let kind = match file.mime_type.as_str() {
        FOLDER_MIME => ItemKind::Folder,
        SHORTCUT_MIME => ItemKind::Shortcut,
        mime if mime.starts_with(GOOGLE_APPS_MIME_PREFIX) => match file.web_view_link {
            Some(url) => ItemKind::GoogleDoc { url },
            None => ItemKind::NotDownloadable,
        },
        _ if !can_download => ItemKind::NotDownloadable,
        _ => ItemKind::File {
            size: number(file.size)?.unwrap_or(0),
            md5: file.md5_checksum,
        },
    };
    let modified = match file.modified_time {
        Some(time) => humantime::parse_rfc3339(&time).map_err(PortError::new)?,
        None => SystemTime::UNIX_EPOCH,
    };
    Ok(DriveItem {
        id: DriveItemId::new(file.id),
        name: file.name,
        kind,
        modified,
        parents: file.parents.into_iter().map(DriveItemId::new).collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn parses_about_response() {
        let about = serde_json::from_str(
            r#"{"user":{"displayName":"Ana Souza","emailAddress":"Ana@Example.com","permissionId":"0123"},
                 "storageQuota":{"limit":"16106127360","usage":"5368709120"}}"#,
        )
        .unwrap();
        let info = about_to_info(about).unwrap();
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
        let about = serde_json::from_str(
            r#"{"user":{"emailAddress":"a@x.com","permissionId":"1"},"storageQuota":{"usage":"10"}}"#,
        )
        .unwrap();
        assert_eq!(about_to_info(about).unwrap().quota.limit, None);
    }

    fn item(json: &str) -> DriveItem {
        file_to_item(serde_json::from_str(json).unwrap()).unwrap()
    }

    #[test]
    fn maps_mime_types_to_item_kinds() {
        let folder = item(
            r#"{"id":"f","name":"Docs","mimeType":"application/vnd.google-apps.folder","parents":["root"]}"#,
        );
        assert_eq!(folder.kind, ItemKind::Folder);
        assert_eq!(folder.parents, [DriveItemId::new("root")]);

        let file = item(
            r#"{"id":"1","name":"a.pdf","mimeType":"application/pdf","size":"42","md5Checksum":"abc",
                "modifiedTime":"2024-05-01T12:00:00.500Z"}"#,
        );
        assert_eq!(
            file.kind,
            ItemKind::File {
                size: 42,
                md5: Some("abc".into())
            }
        );
        assert_eq!(
            file.modified,
            SystemTime::UNIX_EPOCH + Duration::from_millis(1_714_564_800_500)
        );

        let doc = item(
            r#"{"id":"2","name":"Budget","mimeType":"application/vnd.google-apps.spreadsheet",
                "webViewLink":"https://docs.google.com/spreadsheets/d/2"}"#,
        );
        assert_eq!(
            doc.kind,
            ItemKind::GoogleDoc {
                url: "https://docs.google.com/spreadsheets/d/2".into()
            }
        );

        let shortcut =
            item(r#"{"id":"3","name":"s","mimeType":"application/vnd.google-apps.shortcut"}"#);
        assert_eq!(shortcut.kind, ItemKind::Shortcut);

        let locked = item(
            r#"{"id":"4","name":"x.zip","mimeType":"application/zip","capabilities":{"canDownload":false}}"#,
        );
        assert_eq!(locked.kind, ItemKind::NotDownloadable);
    }

    #[test]
    fn ids_cannot_break_out_of_queries_or_paths() {
        assert_eq!(escape_query(r"a'b\c"), r"a\'b\\c");
        assert_eq!(
            endpoint(&["files", "../about"]).path(),
            "/drive/v3/files/..%2Fabout"
        );
    }
}
