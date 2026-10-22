//! Composition root shared by the front-ends: builds the concrete adapters.
//! Each failure is returned as a localized message, ready to show.

use nextgdrive_adapter_config::TomlAccountRepository;
use nextgdrive_adapter_google::{
    CallbackPage, ClientConfig, ClientConfigError, GoogleAuthenticator, GoogleDrive, http_client,
};
use nextgdrive_adapter_keyring::KeyringCredentialStore;
use nextgdrive_application::use_cases::{
    AddAccount, GetAccountStatus, ListAccounts, RemoveAccount,
};
use nextgdrive_i18n::Localizer;

/// A sign-in started by [`Services::add_account`], waiting for the user.
pub type PendingSignIn = nextgdrive_adapter_google::GooglePendingAuthorization;

pub const OAUTH_CLIENT_FILE: &str = "oauth-client.json";

pub fn accounts(i18n: &Localizer) -> Result<TomlAccountRepository, String> {
    TomlAccountRepository::in_config_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))
}

pub async fn credentials(i18n: &Localizer) -> Result<KeyringCredentialStore, String> {
    KeyringCredentialStore::new()
        .await
        .map_err(|e| i18n.tr_args("error-credentials", [("details", e.to_string().into())]))
}

pub fn google(i18n: &Localizer) -> Result<(GoogleAuthenticator, GoogleDrive), String> {
    let dir =
        nextgdrive_adapter_config::config_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    let config = ClientConfig::load(&dir.join(OAUTH_CLIENT_FILE))
        .map_err(|e| client_config_error(i18n, &e))?;
    let page = CallbackPage {
        success: i18n.tr("auth-callback-success"),
        failure: i18n.tr("auth-callback-failure"),
    };
    let http = http_client();
    Ok((
        GoogleAuthenticator::new(config, http.clone(), page),
        GoogleDrive::new(http),
    ))
}

fn client_config_error(i18n: &Localizer, error: &ClientConfigError) -> String {
    use nextgdrive_adapter_google::{CLIENT_ID_VAR, CLIENT_SECRET_VAR};
    match error {
        ClientConfigError::NotFound(path) => i18n.tr_args(
            "error-oauth-client-missing",
            [
                ("path", path.display().to_string().into()),
                ("id_var", CLIENT_ID_VAR.into()),
                ("secret_var", CLIENT_SECRET_VAR.into()),
            ],
        ),
        ClientConfigError::Unreadable(path, details) => i18n.tr_args(
            "error-oauth-client-unreadable",
            [
                ("path", path.display().to_string().into()),
                ("details", details.clone().into()),
            ],
        ),
    }
}

/// Every adapter, for long-running front-ends (the GUI) that need them all.
pub struct Services {
    pub auth: GoogleAuthenticator,
    pub drive: GoogleDrive,
    pub credentials: KeyringCredentialStore,
    pub accounts: TomlAccountRepository,
}

impl Services {
    pub async fn new(i18n: &Localizer) -> Result<Self, String> {
        let accounts = accounts(i18n)?;
        let (auth, drive) = google(i18n)?;
        let credentials = credentials(i18n).await?;
        Ok(Self {
            auth,
            drive,
            credentials,
            accounts,
        })
    }

    pub fn list_accounts(&self) -> ListAccounts<'_, TomlAccountRepository> {
        ListAccounts::new(&self.accounts)
    }

    pub fn add_account(
        &self,
    ) -> AddAccount<
        '_,
        GoogleAuthenticator,
        GoogleDrive,
        KeyringCredentialStore,
        TomlAccountRepository,
    > {
        AddAccount::new(&self.auth, &self.drive, &self.credentials, &self.accounts)
    }

    pub fn account_status(
        &self,
    ) -> GetAccountStatus<
        '_,
        GoogleAuthenticator,
        GoogleDrive,
        KeyringCredentialStore,
        TomlAccountRepository,
    > {
        GetAccountStatus::new(&self.auth, &self.drive, &self.credentials, &self.accounts)
    }

    pub fn remove_account(
        &self,
    ) -> RemoveAccount<'_, KeyringCredentialStore, TomlAccountRepository> {
        RemoveAccount::new(&self.credentials, &self.accounts)
    }
}
