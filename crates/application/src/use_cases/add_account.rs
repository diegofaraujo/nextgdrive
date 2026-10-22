use crate::AppError;
use crate::ports::{
    AccountRepository, Authenticator, CredentialStore, DriveAccountInfo, DriveService,
};

/// Links a Google account, in two steps so front-ends can show the sign-in
/// URL: [`start`](Self::start), then [`finish`](Self::finish).
///
/// Adding an account that is already linked refreshes its stored details and credentials.
pub struct AddAccount<'a, A, D, C, R> {
    auth: &'a A,
    drive: &'a D,
    credentials: &'a C,
    accounts: &'a R,
}

impl<'a, A, D, C, R> AddAccount<'a, A, D, C, R>
where
    A: Authenticator,
    D: DriveService,
    C: CredentialStore,
    R: AccountRepository,
{
    pub fn new(auth: &'a A, drive: &'a D, credentials: &'a C, accounts: &'a R) -> Self {
        Self {
            auth,
            drive,
            credentials,
            accounts,
        }
    }

    /// The user must then open [`url`](crate::ports::PendingAuthorization::url) to sign in.
    pub async fn start(&self) -> Result<A::Pending, AppError> {
        self.auth.start().await.map_err(AppError::Auth)
    }

    /// Waits for the user to sign in, then stores the account and its credentials.
    pub async fn finish(&self, pending: A::Pending) -> Result<DriveAccountInfo, AppError> {
        let tokens = self.auth.finish(pending).await.map_err(AppError::Auth)?;
        let info = self
            .drive
            .account_info(&tokens.access)
            .await
            .map_err(AppError::Drive)?;
        self.credentials
            .store(&info.account.id, &tokens.refresh)
            .await
            .map_err(AppError::Credentials)?;
        self.accounts
            .save(&info.account)
            .map_err(AppError::Storage)?;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{PendingAuthorization, RefreshToken};
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, InMemoryAccounts, InMemoryCredentials, account,
    };

    #[tokio::test]
    async fn links_account_after_sign_in() {
        let (auth, drive) = (FakeAuth, FakeDrive::new(account("1", "ana@x.com")));
        let (credentials, repo) = (InMemoryCredentials::default(), InMemoryAccounts::default());
        let add = AddAccount::new(&auth, &drive, &credentials, &repo);

        let pending = add.start().await.unwrap();
        assert_eq!(pending.url(), FakeAuth::URL);
        let info = add.finish(pending).await.unwrap();

        assert_eq!(info.account, account("1", "ana@x.com"));
        assert_eq!(repo.list().unwrap(), [account("1", "ana@x.com")]);
        assert_eq!(
            credentials.token("1"),
            Some(RefreshToken::new(FakeAuth::REFRESH_TOKEN))
        );
    }
}
