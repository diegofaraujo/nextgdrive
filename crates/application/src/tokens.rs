use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nextgdrive_domain::Account;

use crate::AppError;
use crate::ports::{AccessToken, Authenticator, CredentialStore};

/// Renew tokens this long before Google says they expire, so a token never
/// expires in the middle of a request.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// Hands out access tokens for accounts, refreshing them only when needed.
/// Shared by all use cases that call Google APIs.
pub struct TokenProvider<A, C> {
    auth: A,
    credentials: C,
    cache: Mutex<HashMap<nextgdrive_domain::AccountId, (AccessToken, Instant)>>,
}

impl<A: Authenticator, C: CredentialStore> TokenProvider<A, C> {
    pub fn new(auth: A, credentials: C) -> Self {
        Self {
            auth,
            credentials,
            cache: Mutex::default(),
        }
    }

    pub fn auth(&self) -> &A {
        &self.auth
    }

    pub fn credentials(&self) -> &C {
        &self.credentials
    }

    pub async fn access_token(&self, account: &Account) -> Result<AccessToken, AppError> {
        if let Some((token, valid_until)) =
            self.cache.lock().expect("lock poisoned").get(&account.id)
            && Instant::now() < *valid_until
        {
            return Ok(token.clone());
        }
        let refresh = self
            .credentials
            .load(&account.id)
            .await
            .map_err(AppError::Credentials)?
            .ok_or_else(|| AppError::MissingCredentials(account.email.clone()))?;
        let grant = self.auth.refresh(&refresh).await.map_err(AppError::Auth)?;
        let valid_until = Instant::now() + grant.expires_in.saturating_sub(EXPIRY_MARGIN);
        self.cache
            .lock()
            .expect("lock poisoned")
            .insert(account.id.clone(), (grant.token.clone(), valid_until));
        Ok(grant.token)
    }

    /// Forgets the cached token, e.g. after the account is removed.
    pub fn forget(&self, account: &Account) {
        self.cache
            .lock()
            .expect("lock poisoned")
            .remove(&account.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{FakeAuth, InMemoryCredentials, account};

    #[tokio::test]
    async fn caches_token_until_expiry() {
        let tokens = TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
        );
        let ana = account("1", "ana@x.com");

        tokens.access_token(&ana).await.unwrap();
        tokens.access_token(&ana).await.unwrap();
        assert_eq!(tokens.auth().refresh_count(), 1);

        tokens.forget(&ana);
        tokens.access_token(&ana).await.unwrap();
        assert_eq!(tokens.auth().refresh_count(), 2);
    }

    #[tokio::test]
    async fn fails_without_credentials() {
        let tokens = TokenProvider::new(FakeAuth::default(), InMemoryCredentials::default());
        let result = tokens.access_token(&account("1", "ana@x.com")).await;
        assert!(matches!(result, Err(AppError::MissingCredentials(_))));
    }
}
