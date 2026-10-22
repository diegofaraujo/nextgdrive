use std::time::Duration;

use nextgdrive_application::ports::{
    AccessGrant, AccessToken, Authenticator, AuthorizedTokens, PendingAuthorization, PortError,
    RefreshToken,
};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::ClientConfig;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Full Drive access, needed to sync files in both directions.
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
/// How long to wait for the user to finish signing in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_secs(60 * 60);

type OAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Page shown in the browser after sign-in, so the user knows to return to the app.
/// Front-ends provide it already translated.
#[derive(Clone)]
pub struct CallbackPage {
    pub success: String,
    pub failure: String,
}

/// OAuth 2.0 for installed apps: authorization code with PKCE, received on a
/// loopback HTTP server.
/// See <https://developers.google.com/identity/protocols/oauth2/native-app>.
pub struct GoogleAuthenticator {
    client: OAuthClient,
    http: oauth2::reqwest::Client,
    page: CallbackPage,
}

pub struct GooglePendingAuthorization {
    url: String,
    listener: TcpListener,
    redirect: RedirectUrl,
    csrf: CsrfToken,
    verifier: PkceCodeVerifier,
}

impl PendingAuthorization for GooglePendingAuthorization {
    fn url(&self) -> &str {
        &self.url
    }
}

impl GoogleAuthenticator {
    pub fn new(config: ClientConfig, http: oauth2::reqwest::Client, page: CallbackPage) -> Self {
        let client = BasicClient::new(ClientId::new(config.client_id))
            .set_client_secret(ClientSecret::new(config.client_secret))
            .set_auth_uri(AuthUrl::new(AUTH_URL.to_owned()).expect("valid URL"))
            .set_token_uri(TokenUrl::new(TOKEN_URL.to_owned()).expect("valid URL"));
        Self { client, http, page }
    }
}

impl Authenticator for GoogleAuthenticator {
    type Pending = GooglePendingAuthorization;

    async fn start(&self) -> Result<GooglePendingAuthorization, PortError> {
        // Port 0: the OS picks a free port; Google accepts any loopback port.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(PortError::new)?;
        let port = listener.local_addr().map_err(PortError::new)?.port();
        let redirect =
            RedirectUrl::new(format!("http://127.0.0.1:{port}")).map_err(PortError::new)?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, csrf) = self
            .client
            .clone()
            .set_redirect_uri(redirect.clone())
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new(DRIVE_SCOPE.to_owned()))
            .set_pkce_challenge(challenge)
            // Without these, Google only returns a refresh token on first consent.
            .add_extra_param("access_type", "offline")
            .add_extra_param("prompt", "consent")
            .url();
        Ok(GooglePendingAuthorization {
            url: url.into(),
            listener,
            redirect,
            csrf,
            verifier,
        })
    }

    async fn finish(
        &self,
        pending: GooglePendingAuthorization,
    ) -> Result<AuthorizedTokens, PortError> {
        let code = tokio::time::timeout(SIGN_IN_TIMEOUT, self.receive_code(&pending))
            .await
            .map_err(|_| PortError::new("timed out waiting for sign-in"))??;
        let response = self
            .client
            .clone()
            .set_redirect_uri(pending.redirect)
            .exchange_code(code)
            .set_pkce_verifier(pending.verifier)
            .request_async(&self.http)
            .await
            .map_err(|e| PortError::new(format!("token exchange failed: {e}")))?;
        let refresh = response
            .refresh_token()
            .ok_or_else(|| PortError::new("Google did not return a refresh token"))?;
        Ok(AuthorizedTokens {
            access: AccessToken::new(response.access_token().secret()),
            refresh: RefreshToken::new(refresh.secret()),
        })
    }

    async fn refresh(&self, token: &RefreshToken) -> Result<AccessGrant, PortError> {
        let response = self
            .client
            .exchange_refresh_token(&oauth2::RefreshToken::new(token.secret().to_owned()))
            .request_async(&self.http)
            .await
            .map_err(|e| PortError::new(format!("token refresh failed: {e}")))?;
        Ok(AccessGrant {
            token: AccessToken::new(response.access_token().secret()),
            // Google access tokens last one hour; it always sends `expires_in`.
            expires_in: response.expires_in().unwrap_or(DEFAULT_TOKEN_LIFETIME),
        })
    }
}

impl GoogleAuthenticator {
    /// Serves the loopback redirect until Google calls it with a code or an error.
    async fn receive_code(
        &self,
        pending: &GooglePendingAuthorization,
    ) -> Result<AuthorizationCode, PortError> {
        loop {
            let (mut stream, _) = pending.listener.accept().await.map_err(PortError::new)?;
            let Some(query) = read_request_query(&mut stream).await else {
                // Not the redirect (e.g. a favicon request).
                respond(&mut stream, "404 Not Found", "").await;
                continue;
            };
            let result = parse_redirect(&query, pending.csrf.secret());
            let (status, message) = match &result {
                Ok(_) => ("200 OK", &self.page.success),
                Err(_) => ("400 Bad Request", &self.page.failure),
            };
            respond(&mut stream, status, &html_page(message)).await;
            return result;
        }
    }
}

/// Returns the query string of an HTTP `GET /?...` request.
async fn read_request_query(stream: &mut TcpStream) -> Option<String> {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await.ok()?;
    let target = line.strip_prefix("GET ")?.split(' ').next()?;
    let query = target.strip_prefix("/?")?;
    Some(query.to_owned())
}

fn parse_redirect(query: &str, expected_state: &str) -> Result<AuthorizationCode, PortError> {
    let url = Url::parse(&format!("http://localhost/?{query}")).map_err(PortError::new)?;
    let param = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    if let Some(error) = param("error") {
        return Err(PortError::new(format!(
            "sign-in was not completed: {error}"
        )));
    }
    if param("state").as_deref() != Some(expected_state) {
        return Err(PortError::new("sign-in response has an invalid state"));
    }
    param("code")
        .map(AuthorizationCode::new)
        .ok_or_else(|| PortError::new("sign-in response has no code"))
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The browser may have gone away; the sign-in result is what matters.
    let _ = stream.write_all(response.as_bytes()).await;
}

fn html_page(message: &str) -> String {
    let message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Next GDrive</title></head>\
         <body style=\"font-family:sans-serif;text-align:center;margin-top:4em\"><h1>Next GDrive</h1><p>{message}</p></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_code_with_matching_state() {
        let code = parse_redirect("state=abc&code=4%2F0xyz&scope=drive", "abc").unwrap();
        assert_eq!(code.secret(), "4/0xyz");
    }

    #[test]
    fn rejects_wrong_state_and_errors() {
        assert!(parse_redirect("state=evil&code=x", "abc").is_err());
        assert!(parse_redirect("error=access_denied&state=abc", "abc").is_err());
        assert!(parse_redirect("state=abc", "abc").is_err());
    }

    #[test]
    fn page_escapes_message() {
        assert!(html_page("<b>&").contains("&lt;b&gt;&amp;"));
    }

    #[tokio::test]
    async fn loopback_receives_code_and_ignores_other_requests() {
        let auth = GoogleAuthenticator::new(
            ClientConfig {
                client_id: "id".into(),
                client_secret: "secret".into(),
            },
            crate::http_client(),
            CallbackPage {
                success: "ok".into(),
                failure: "failed".into(),
            },
        );
        let pending = auth.start().await.unwrap();
        let url = Url::parse(pending.url()).unwrap();
        let param = |name| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .unwrap()
                .1
                .into_owned()
        };
        assert_eq!(param("scope"), DRIVE_SCOPE);
        assert_eq!(param("code_challenge_method"), "S256");
        let (redirect, state) = (param("redirect_uri"), param("state"));

        let browser = tokio::spawn(async move {
            let http = oauth2::reqwest::Client::new();
            let favicon = http
                .get(format!("{redirect}/favicon.ico"))
                .send()
                .await
                .unwrap();
            assert_eq!(favicon.status(), 404);
            let page = http
                .get(format!("{redirect}/?state={state}&code=the-code"))
                .send()
                .await
                .unwrap();
            assert_eq!(page.status(), 200);
            assert!(page.text().await.unwrap().contains("ok"));
        });

        let code = auth.receive_code(&pending).await.unwrap();
        browser.await.unwrap();
        assert_eq!(code.secret(), "the-code");
    }
}
