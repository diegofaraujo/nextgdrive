//! Composition root shared by the front-ends: builds the concrete adapters.
//! Each failure is returned as a localized message, ready to show.

pub mod host;

use std::collections::BTreeMap;
use std::path::PathBuf;

use nextgdrive_adapter_config::{TomlAccountRepository, TomlSettingsRepository};
use nextgdrive_adapter_google::{
    CLIENT_ID_VAR, CLIENT_SECRET_VAR, CallbackPage, ClientConfig, ClientConfigError,
    GoogleAuthenticator, GoogleDrive, http_client,
};
use nextgdrive_adapter_keyring::KeyringCredentialStore;
use nextgdrive_adapter_localfs::Filesystem;
use nextgdrive_adapter_sqlite::SqliteSyncState;
use nextgdrive_application::TokenProvider;
use nextgdrive_application::background::BackgroundSync;
use nextgdrive_application::use_cases::{
    AddAccount, BrowseDrive, ChangeSelection, GetAccountStatus, GetSettings, ListAccounts,
    RemoveAccount, SetSyncRoot, SyncAccount,
};
use nextgdrive_domain::Settings;
use nextgdrive_i18n::Localizer;

/// A sign-in started by [`Services::add_account`], waiting for the user.
pub type PendingSignIn = nextgdrive_adapter_google::GooglePendingAuthorization;

pub type Tokens = TokenProvider<GoogleAuthenticator, KeyringCredentialStore>;

pub const OAUTH_CLIENT_FILE: &str = "oauth-client.json";
const STATE_FILE: &str = "state.sqlite3";
/// Default sync folder, inside the home directory.
const DEFAULT_SYNC_DIR: &str = "GoogleDrive";

/// Adapters that only touch local files. Enough for commands that need no
/// network or keyring access.
pub struct LocalServices {
    pub accounts: TomlAccountRepository,
    pub settings: TomlSettingsRepository,
}

impl LocalServices {
    pub fn new(i18n: &Localizer) -> Result<Self, String> {
        let no_config_dir = || i18n.tr("error-no-config-dir");
        Ok(Self {
            accounts: TomlAccountRepository::in_config_dir().ok_or_else(no_config_dir)?,
            settings: TomlSettingsRepository::in_config_dir(default_settings(i18n)?)
                .ok_or_else(no_config_dir)?,
        })
    }

    pub fn list_accounts(&self) -> ListAccounts<'_, TomlAccountRepository> {
        ListAccounts::new(&self.accounts)
    }

    pub fn get_settings(&self) -> GetSettings<'_, TomlSettingsRepository> {
        GetSettings::new(&self.settings)
    }

    pub fn set_sync_root(&self) -> SetSyncRoot<'_, TomlSettingsRepository> {
        SetSyncRoot::new(&self.settings)
    }
}

/// Every adapter, including Google and the keyring.
pub struct Services {
    pub local: LocalServices,
    pub tokens: Tokens,
    pub drive: GoogleDrive,
    pub fs: Filesystem,
    pub state: SqliteSyncState,
}

impl Services {
    pub async fn new(i18n: &Localizer) -> Result<Self, String> {
        let local = LocalServices::new(i18n)?;
        let dir = nextgdrive_adapter_config::config_dir()
            .ok_or_else(|| i18n.tr("error-no-config-dir"))?;
        let config = ClientConfig::load(&dir.join(OAUTH_CLIENT_FILE))
            .map_err(|e| client_config_error(i18n, &e))?;
        let page = CallbackPage {
            success: i18n.tr("auth-callback-success"),
            failure: i18n.tr("auth-callback-failure"),
        };
        let credentials = KeyringCredentialStore::new()
            .await
            .map_err(|e| i18n.tr_args("error-credentials", [("details", e.to_string().into())]))?;
        let data =
            nextgdrive_adapter_config::data_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
        let state = SqliteSyncState::open(&data.join(STATE_FILE))
            .map_err(|e| i18n.tr_args("error-storage", [("details", e.to_string().into())]))?;
        let http = http_client();
        Ok(Self {
            local,
            tokens: TokenProvider::new(
                GoogleAuthenticator::new(config, http.clone(), page),
                credentials,
            ),
            drive: GoogleDrive::new(http),
            fs: Filesystem,
            state,
        })
    }

    pub fn add_account(
        &self,
    ) -> AddAccount<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        TomlAccountRepository,
    > {
        AddAccount::new(&self.tokens, &self.drive, &self.local.accounts)
    }

    pub fn account_status(
        &self,
    ) -> GetAccountStatus<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        TomlAccountRepository,
    > {
        GetAccountStatus::new(&self.tokens, &self.drive, &self.local.accounts)
    }

    pub fn remove_account(
        &self,
    ) -> RemoveAccount<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        TomlAccountRepository,
        TomlSettingsRepository,
    > {
        RemoveAccount::new(&self.tokens, &self.local.accounts, &self.local.settings)
    }

    pub fn browse_drive(
        &self,
    ) -> BrowseDrive<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        TomlSettingsRepository,
    > {
        BrowseDrive::new(&self.tokens, &self.drive, &self.local.settings)
    }

    #[allow(clippy::type_complexity)]
    pub fn change_selection(
        &self,
    ) -> ChangeSelection<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        TomlSettingsRepository,
        Filesystem,
        SqliteSyncState,
    > {
        ChangeSelection::new(
            &self.tokens,
            &self.drive,
            &self.local.settings,
            &self.fs,
            &self.state,
        )
    }
}

impl Services {
    #[allow(clippy::type_complexity)]
    pub fn sync_account(
        &self,
    ) -> SyncAccount<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        Filesystem,
        TomlSettingsRepository,
        SqliteSyncState,
    > {
        SyncAccount::new(
            &self.tokens,
            &self.drive,
            &self.fs,
            &self.local.settings,
            &self.state,
        )
    }
}

impl Services {
    #[allow(clippy::type_complexity)]
    pub fn background_sync(
        &self,
    ) -> BackgroundSync<
        '_,
        GoogleAuthenticator,
        KeyringCredentialStore,
        GoogleDrive,
        Filesystem,
        TomlSettingsRepository,
        SqliteSyncState,
        TomlAccountRepository,
    > {
        BackgroundSync::new(
            &self.tokens,
            &self.drive,
            &self.fs,
            &self.local.settings,
            &self.state,
            &self.local.accounts,
        )
    }
}

/// `~/GoogleDrive`, with folder names in the current language.
fn default_settings(i18n: &Localizer) -> Result<Settings, String> {
    let home = std::env::home_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    Ok(Settings {
        sync_root: home.join(DEFAULT_SYNC_DIR),
        my_drive_folder: i18n.tr("folder-my-drive"),
        shared_drives_folder: i18n.tr("folder-shared-drives"),
        poll_interval_secs: Settings::DEFAULT_POLL_INTERVAL_SECS,
        selections: BTreeMap::new(),
    })
}

fn client_config_error(i18n: &Localizer, error: &ClientConfigError) -> String {
    let path = |p: &PathBuf| p.display().to_string().into();
    match error {
        ClientConfigError::NotFound(p) => i18n.tr_args(
            "error-oauth-client-missing",
            [
                ("path", path(p)),
                ("id_var", CLIENT_ID_VAR.into()),
                ("secret_var", CLIENT_SECRET_VAR.into()),
            ],
        ),
        ClientConfigError::Unreadable(p, details) => i18n.tr_args(
            "error-oauth-client-unreadable",
            [("path", path(p)), ("details", details.clone().into())],
        ),
    }
}
