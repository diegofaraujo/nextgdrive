use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nextgdrive_application::ports::{AccountRepository, PortError};
use nextgdrive_domain::{Account, AccountId, Email};
use serde::{Deserialize, Serialize};

/// Stores accounts in a TOML file. A missing file means no accounts.
pub struct TomlAccountRepository {
    path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct AccountsFile {
    #[serde(default)]
    accounts: Vec<AccountEntry>,
}

#[derive(Serialize, Deserialize)]
struct AccountEntry {
    id: String,
    email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

impl TomlAccountRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Uses `accounts.toml` in the default config directory.
    pub fn in_config_dir() -> Option<Self> {
        crate::config_dir().map(|dir| Self::new(dir.join("accounts.toml")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<Vec<Account>, PortError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(self.error(e)),
        };
        let file: AccountsFile = toml::from_str(&text).map_err(|e| self.error(e))?;
        file.accounts
            .into_iter()
            .map(|entry| {
                Ok(Account {
                    id: AccountId::new(entry.id),
                    email: Email::parse(&entry.email).map_err(|e| self.error(e))?,
                    display_name: entry.display_name,
                })
            })
            .collect()
    }

    fn store(&self, accounts: &[Account]) -> Result<(), PortError> {
        let file = AccountsFile {
            accounts: accounts
                .iter()
                .map(|a| AccountEntry {
                    id: a.id.to_string(),
                    email: a.email.to_string(),
                    display_name: a.display_name.clone(),
                })
                .collect(),
        };
        let text = toml::to_string(&file).map_err(|e| self.error(e))?;
        crate::write_atomically(&self.path, &text).map_err(|e| self.error(e))
    }

    fn error(&self, source: impl std::fmt::Display) -> PortError {
        PortError::new(format!("{}: {source}", self.path.display()))
    }
}

impl AccountRepository for TomlAccountRepository {
    fn list(&self) -> Result<Vec<Account>, PortError> {
        self.load()
    }

    fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError> {
        Ok(self.load()?.into_iter().find(|a| &a.email == email))
    }

    fn save(&self, account: &Account) -> Result<(), PortError> {
        let mut accounts = self.load()?;
        match accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account.clone(),
            None => accounts.push(account.clone()),
        }
        self.store(&accounts)
    }

    fn remove(&self, id: &AccountId) -> Result<(), PortError> {
        let mut accounts = self.load()?;
        accounts.retain(|a| &a.id != id);
        self.store(&accounts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, email: &str, name: Option<&str>) -> Account {
        Account {
            id: AccountId::new(id),
            email: Email::parse(email).unwrap(),
            display_name: name.map(str::to_owned),
        }
    }

    fn repo() -> (tempfile::TempDir, TomlAccountRepository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlAccountRepository::new(dir.path().join("nested/accounts.toml"));
        (dir, repo)
    }

    #[test]
    fn missing_file_means_no_accounts() {
        let (_dir, repo) = repo();
        assert!(repo.list().unwrap().is_empty());
    }

    #[test]
    fn save_round_trips_and_replaces_by_id() {
        let (_dir, repo) = repo();
        repo.save(&account("1", "ana@x.com", Some("Ana"))).unwrap();
        repo.save(&account("2", "zoe@x.com", None)).unwrap();
        repo.save(&account("1", "ana.new@x.com", None)).unwrap();

        assert_eq!(
            repo.list().unwrap(),
            [
                account("1", "ana.new@x.com", None),
                account("2", "zoe@x.com", None)
            ]
        );
        let found = repo
            .find_by_email(&Email::parse("zoe@x.com").unwrap())
            .unwrap();
        assert_eq!(found.map(|a| a.id), Some(AccountId::new("2")));
    }

    #[test]
    fn remove_deletes_only_matching_account() {
        let (_dir, repo) = repo();
        repo.save(&account("1", "ana@x.com", None)).unwrap();
        repo.save(&account("2", "zoe@x.com", None)).unwrap();

        repo.remove(&AccountId::new("1")).unwrap();

        assert_eq!(repo.list().unwrap(), [account("2", "zoe@x.com", None)]);
    }

    #[test]
    fn corrupt_file_reports_its_path() {
        let (_dir, repo) = repo();
        fs::create_dir_all(repo.path().parent().unwrap()).unwrap();
        fs::write(repo.path(), "not = [valid").unwrap();

        let error = repo.list().unwrap_err().to_string();

        assert!(error.contains("accounts.toml"), "{error}");
    }
}
