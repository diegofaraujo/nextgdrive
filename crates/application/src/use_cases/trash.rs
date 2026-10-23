use std::collections::HashMap;
use std::time::SystemTime;

use nextgdrive_domain::{Account, DriveItem, DriveItemId, DriveKind, Settings};

use crate::ports::{Authenticator, CredentialStore, DriveService, SettingsRepository};
use crate::{AppError, TokenProvider};

/// Something in the trash, with where it will go back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    pub item: DriveItem,
    /// Unknown for My Drive items: Drive does not report it.
    pub trashed_at: Option<SystemTime>,
    /// Name of the folder it was in (the drive's local folder name for
    /// top-level items); `None` if that folder is no longer accessible.
    pub location: Option<String>,
}

/// Shows an account's Google Drive trash and restores items from it.
pub struct DriveTrash<'a, A, C, D, S> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    settings: &'a S,
}

impl<'a, A, C, D, S> DriveTrash<'a, A, C, D, S>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    S: SettingsRepository,
{
    pub fn new(tokens: &'a TokenProvider<A, C>, drive: &'a D, settings: &'a S) -> Self {
        Self {
            tokens,
            drive,
            settings,
        }
    }

    /// Most recently trashed first, when Drive says when; otherwise in Drive's
    /// recency order.
    pub async fn list(&self, account: &Account) -> Result<Vec<TrashEntry>, AppError> {
        let token = self.tokens.access_token(account).await?;
        let settings = self.settings.load().map_err(AppError::Storage)?;
        let mut items = self
            .drive
            .trashed_items(&token)
            .await
            .map_err(AppError::Drive)?;
        // Stable: items without a date keep Drive's order, after dated ones.
        items.sort_by_key(|t| std::cmp::Reverse(t.trashed_at));

        let mut names = self.drive_names(&token, &settings).await?;
        let mut entries = Vec::with_capacity(items.len());
        for trashed in items {
            let location = match trashed.item.parents.first() {
                Some(parent) => {
                    if !names.contains_key(parent) {
                        // A parent that is gone (or not shared with us) just has no name.
                        let name = self.drive.item(&token, parent).await.ok().map(|p| p.name);
                        names.insert(parent.clone(), name);
                    }
                    names[parent].clone()
                }
                None => None,
            };
            entries.push(TrashEntry {
                item: trashed.item,
                trashed_at: trashed.trashed_at,
                location,
            });
        }
        Ok(entries)
    }

    /// Puts an item back where it was. If that folder is synced, the next sync
    /// brings it back to this computer too.
    pub async fn restore(
        &self,
        account: &Account,
        id: &DriveItemId,
    ) -> Result<DriveItem, AppError> {
        let token = self.tokens.access_token(account).await?;
        self.drive
            .restore(&token, id)
            .await
            .map_err(AppError::Drive)
    }

    /// Drive roots are shown with the names used for their local folders.
    async fn drive_names(
        &self,
        token: &crate::ports::AccessToken,
        settings: &Settings,
    ) -> Result<HashMap<DriveItemId, Option<String>>, AppError> {
        let drives = self.drive.drives(token).await.map_err(AppError::Drive)?;
        Ok(drives
            .into_iter()
            .map(|d| {
                let name = match d.kind {
                    DriveKind::MyDrive => settings.my_drive_folder.clone(),
                    DriveKind::Shared { name } => name,
                };
                (d.root, Some(name))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, InMemoryCredentials, InMemorySettings, account,
    };

    fn setup() -> (
        Account,
        TokenProvider<FakeAuth, InMemoryCredentials>,
        FakeDrive,
        InMemorySettings,
    ) {
        let ana = account("1", "ana@x.com");
        let tokens = TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
        );
        let drive = FakeDrive::new(ana.clone())
            .with_folder("root", "docs", "Docs")
            .with_file("docs", "a", "a.txt")
            .with_file("root", "b", "b.txt");
        (ana, tokens, drive, InMemorySettings::default())
    }

    #[tokio::test]
    async fn lists_trashed_items_newest_first_with_their_folder() {
        let (ana, tokens, drive, settings) = setup();
        drive.remove("a");
        drive.remove("b");

        let entries = DriveTrash::new(&tokens, &drive, &settings)
            .list(&ana)
            .await
            .unwrap();

        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.item.name.as_str(), e.location.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [("b.txt", Some("My Drive")), ("a.txt", Some("Docs"))]
        );
    }

    #[tokio::test]
    async fn a_trashed_folder_is_one_entry_and_restores_with_its_content() {
        let (ana, tokens, drive, settings) = setup();
        drive.remove("docs");
        let trash = DriveTrash::new(&tokens, &drive, &settings);

        let entries = trash.list(&ana).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item.name, "Docs");

        trash
            .restore(&ana, &DriveItemId::new("docs"))
            .await
            .unwrap();

        assert!(drive.has("docs") && drive.has("a"));
        assert!(trash.list(&ana).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restoring_something_not_in_the_trash_fails() {
        let (ana, tokens, drive, settings) = setup();
        let result = DriveTrash::new(&tokens, &drive, &settings)
            .restore(&ana, &DriveItemId::new("a"))
            .await;
        assert!(matches!(result, Err(AppError::Drive(_))));
    }
}
