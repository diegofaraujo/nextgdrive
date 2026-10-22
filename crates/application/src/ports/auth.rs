use std::fmt;
use std::future::Future;

use super::PortError;

/// Long-lived OAuth token used to obtain access tokens. Must be kept secret.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshToken(String);

/// Short-lived OAuth token sent with each API request.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

macro_rules! secret_string {
    ($name:ident) => {
        impl $name {
            pub fn new(token: impl Into<String>) -> Self {
                Self(token.into())
            }

            pub fn secret(&self) -> &str {
                &self.0
            }
        }

        /// Never prints the token, so it cannot leak into logs.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

secret_string!(RefreshToken);
secret_string!(AccessToken);

#[derive(Debug, Clone)]
pub struct AuthorizedTokens {
    pub access: AccessToken,
    pub refresh: RefreshToken,
}

/// An authorization started with [`Authenticator::start`], waiting for the
/// user to sign in.
pub trait PendingAuthorization: Send {
    /// URL the user must open in a browser to sign in.
    fn url(&self) -> &str;
}

/// Signs users in to their Google account (OAuth 2.0).
pub trait Authenticator: Sync {
    type Pending: PendingAuthorization;

    fn start(&self) -> impl Future<Output = Result<Self::Pending, PortError>> + Send;

    /// Waits for the user to complete sign-in in the browser.
    fn finish(
        &self,
        pending: Self::Pending,
    ) -> impl Future<Output = Result<AuthorizedTokens, PortError>> + Send;

    fn refresh(
        &self,
        token: &RefreshToken,
    ) -> impl Future<Output = Result<AccessToken, PortError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_redacted_in_debug_output() {
        assert_eq!(
            format!("{:?}", RefreshToken::new("1//secret")),
            "RefreshToken(<redacted>)"
        );
        assert_eq!(
            format!("{:?}", AccessToken::new("ya29.secret")),
            "AccessToken(<redacted>)"
        );
    }
}
