use nextgdrive_domain::{Account, Drive, DriveItem, DriveItemId, ItemKind};

use crate::ports::{Authenticator, CredentialStore, DriveService, SettingsRepository};
use crate::{AppError, TokenProvider};

/// Something the user can choose to sync, with whether it is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseEntry<T> {
    pub item: T,
    pub selected: bool,
}

/// Lets the user explore an account's drives and folders to choose what to sync.
pub struct BrowseDrive<'a, A, C, D, S> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    settings: &'a S,
}

impl<'a, A, C, D, S> BrowseDrive<'a, A, C, D, S>
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

    /// My Drive first, then shared drives.
    pub async fn drives(&self, account: &Account) -> Result<Vec<BrowseEntry<Drive>>, AppError> {
        let token = self.tokens.access_token(account).await?;
        let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
        let settings = self.settings.load().map_err(AppError::Storage)?;
        Ok(drives
            .into_iter()
            .map(|drive| BrowseEntry {
                selected: settings.is_selected(&account.id, &drive.root),
                item: drive,
            })
            .collect())
    }

    /// Subfolders of a folder (or of a drive root), sorted by name.
    pub async fn folders(
        &self,
        account: &Account,
        parent: &DriveItemId,
    ) -> Result<Vec<BrowseEntry<DriveItem>>, AppError> {
        let token = self.tokens.access_token(account).await?;
        let mut children = self
            .drive
            .children(&token, parent)
            .await
            .map_err(AppError::Drive)?;
        children.retain(|item| item.kind == ItemKind::Folder);
        children.sort_by_cached_key(|item| item.name.to_lowercase());
        let settings = self.settings.load().map_err(AppError::Storage)?;
        Ok(children
            .into_iter()
            .map(|item| BrowseEntry {
                selected: settings.is_selected(&account.id, &item.id),
                item,
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

    #[tokio::test]
    async fn lists_subfolders_sorted_with_selection() {
        let ana = account("1", "ana@x.com");
        let tokens = TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
        );
        let drive = FakeDrive::new(ana.clone())
            .with_folder("root", "b-id", "beta")
            .with_folder("root", "a-id", "Alpha")
            .with_file("root", "f-id", "file.txt");
        let settings = InMemorySettings::default();
        settings.update(|s| s.select(&ana.id, DriveItemId::new("b-id")));

        let entries = BrowseDrive::new(&tokens, &drive, &settings)
            .folders(&ana, &DriveItemId::new("root"))
            .await
            .unwrap();

        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.item.name.as_str(), e.selected))
            .collect();
        assert_eq!(summary, [("Alpha", false), ("beta", true)]);
    }
}
