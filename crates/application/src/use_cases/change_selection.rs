use nextgdrive_domain::{Account, DriveItemId, ItemKind};

use crate::ports::{Authenticator, CredentialStore, DriveService, SettingsRepository};
use crate::{AppError, TokenProvider};

/// Chooses which folders of an account are synced.
pub struct ChangeSelection<'a, A, C, D, S> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    settings: &'a S,
}

impl<'a, A, C, D, S> ChangeSelection<'a, A, C, D, S>
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

    /// Selects a folder or a drive root (whose id is also a folder in Drive).
    pub async fn select(&self, account: &Account, folder: &DriveItemId) -> Result<(), AppError> {
        let token = self.tokens.access_token(account).await?;
        let item = self
            .drive
            .item(&token, folder)
            .await
            .map_err(AppError::Drive)?;
        if item.kind != ItemKind::Folder {
            return Err(AppError::NotAFolder(folder.clone()));
        }
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        // Drive resolves aliases such as "root" to the real id; store that.
        settings.select(&account.id, item.id);
        self.settings.save(&settings).map_err(AppError::Storage)
    }

    pub fn deselect(&self, account: &Account, folder: &DriveItemId) -> Result<(), AppError> {
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        settings.deselect(&account.id, folder);
        self.settings.save(&settings).map_err(AppError::Storage)
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
            .with_file("root", "f", "file.txt");
        (ana, tokens, drive, InMemorySettings::default())
    }

    #[tokio::test]
    async fn selects_and_deselects_folders() {
        let (ana, tokens, drive, settings) = setup();
        let change = ChangeSelection::new(&tokens, &drive, &settings);

        change
            .select(&ana, &DriveItemId::new("docs"))
            .await
            .unwrap();
        assert!(
            settings
                .load()
                .unwrap()
                .is_selected(&ana.id, &DriveItemId::new("docs"))
        );

        change.deselect(&ana, &DriveItemId::new("docs")).unwrap();
        assert!(settings.load().unwrap().selections.is_empty());
    }

    #[tokio::test]
    async fn refuses_to_select_files() {
        let (ana, tokens, drive, settings) = setup();
        let result = ChangeSelection::new(&tokens, &drive, &settings)
            .select(&ana, &DriveItemId::new("f"))
            .await;
        assert!(matches!(result, Err(AppError::NotAFolder(_))));
    }
}
