//! Credential storage in the system keyring, through the Secret Service API
//! (GNOME Keyring, KWallet) or the secret portal inside Flatpak.

use nextgdrive_application::ports::{CredentialStore, PortError, RefreshToken};
use nextgdrive_domain::AccountId;

const APPLICATION: &str = "io.github.diegofaraujo.NextGDrive";

pub struct KeyringCredentialStore {
    keyring: oo7::Keyring,
}

impl KeyringCredentialStore {
    /// Connects to the keyring, asking the user to unlock it if needed.
    pub async fn new() -> Result<Self, PortError> {
        let keyring = oo7::Keyring::new().await.map_err(PortError::new)?;
        keyring.unlock().await.map_err(PortError::new)?;
        Ok(Self { keyring })
    }
}

fn attributes(account: &AccountId) -> [(&'static str, &str); 2] {
    [
        ("application", APPLICATION),
        ("account-id", account.as_str()),
    ]
}

impl CredentialStore for KeyringCredentialStore {
    async fn store(&self, account: &AccountId, token: &RefreshToken) -> Result<(), PortError> {
        // The label is what users see in password managers such as Seahorse.
        let label = format!("Next GDrive ({account})");
        self.keyring
            .create_item(&label, &attributes(account), token.secret(), true)
            .await
            .map_err(PortError::new)
    }

    async fn load(&self, account: &AccountId) -> Result<Option<RefreshToken>, PortError> {
        let items = self
            .keyring
            .search_items(&attributes(account))
            .await
            .map_err(PortError::new)?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item.secret().await.map_err(PortError::new)?;
        let token = std::str::from_utf8(secret.as_bytes()).map_err(PortError::new)?;
        Ok(Some(RefreshToken::new(token)))
    }

    async fn delete(&self, account: &AccountId) -> Result<(), PortError> {
        self.keyring
            .delete(&attributes(account))
            .await
            .map_err(PortError::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the real keyring of the current session.
    #[tokio::test]
    #[ignore = "needs a running Secret Service; run with --ignored"]
    async fn stores_loads_and_deletes_token() {
        let store = KeyringCredentialStore::new().await.unwrap();
        let account = AccountId::new(format!("test-{}", std::process::id()));

        store
            .store(&account, &RefreshToken::new("first"))
            .await
            .unwrap();
        store
            .store(&account, &RefreshToken::new("second"))
            .await
            .unwrap();
        assert_eq!(
            store.load(&account).await.unwrap(),
            Some(RefreshToken::new("second"))
        );

        store.delete(&account).await.unwrap();
        assert_eq!(store.load(&account).await.unwrap(), None);
        store.delete(&account).await.unwrap();
    }
}
