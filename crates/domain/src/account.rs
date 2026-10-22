use std::fmt;

use crate::DomainError;

/// Stable identifier Google assigns to a user. Unlike the email, it never
/// changes, so it is the account's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A normalized (lowercase) email address.
///
/// Besides basic shape, it rejects characters that cannot appear in a
/// file name, because the email is used as the account's folder name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let email = raw.trim().to_lowercase();
        let valid = match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && !domain.is_empty()
                    && !domain.contains('@')
                    && !email.contains(['/', '\0'])
                    && !email.chars().any(char::is_whitespace)
            }
            None => false,
        };
        if valid {
            Ok(Self(email))
        } else {
            Err(DomainError::InvalidEmail(raw.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A Google account linked to Next GDrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub email: Email,
    pub display_name: Option<String>,
}

impl Account {
    /// Name of the subfolder, under the sync root, that holds this account's files.
    pub fn folder_name(&self) -> &str {
        self.email.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_trimmed_and_lowercased() {
        let email = Email::parse("  Jane.Doe@Example.com ").unwrap();
        assert_eq!(email.as_str(), "jane.doe@example.com");
    }

    #[test]
    fn email_rejects_invalid_shapes() {
        for raw in [
            "",
            "jane",
            "@example.com",
            "jane@",
            "a@b@c",
            "ja ne@x.com",
            "ja/ne@x.com",
        ] {
            assert_eq!(
                Email::parse(raw),
                Err(DomainError::InvalidEmail(raw.to_owned())),
                "{raw:?} should be invalid"
            );
        }
    }

    #[test]
    fn folder_name_is_the_email() {
        let account = Account {
            id: AccountId::new("123"),
            email: Email::parse("jane@example.com").unwrap(),
            display_name: None,
        };
        assert_eq!(account.folder_name(), "jane@example.com");
    }
}
