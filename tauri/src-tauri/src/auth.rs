//! OAuth2 (Authorization Code + PKCE) against the Microsoft identity platform.
//!
//! We obtain an access token scoped for IMAP and a long-lived refresh token.
//! The access token is later fed to IMAP via the `XOAUTH2` SASL mechanism —
//! Microsoft no longer permits basic auth (username/password) for IMAP.
//!
//! Desktop apps use a loopback redirect (`http://localhost:<port>`). Azure
//! treats any loopback port as valid for the "Mobile and desktop applications"
//! platform, so registering `http://localhost` once is enough.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use oauth2::basic::{
    BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
    BasicTokenType,
};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, Client, ClientId, CsrfToken, ExtraTokenFields,
    PkceCodeChallenge, RedirectUrl, RefreshToken, Scope, StandardRevocableToken,
    StandardTokenResponse, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::{self, AppConfig};

/// Scopes: IMAP access for Outlook, plus offline_access for a refresh token and
/// openid/profile so we can read the user's email from the id_token.
const SCOPES: &[&str] = &[
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "offline_access",
    "openid",
    "profile",
];

/// Extra token fields so we can capture the OIDC `id_token` Microsoft returns.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct MsExtraFields {
    id_token: Option<String>,
}
impl ExtraTokenFields for MsExtraFields {}

type MsTokenResponse = StandardTokenResponse<MsExtraFields, BasicTokenType>;
type MsClient = Client<
    BasicErrorResponse,
    MsTokenResponse,
    BasicTokenType,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
>;

fn build_client(cfg: &AppConfig, redirect: Option<String>) -> Result<MsClient> {
    if cfg.client_id.trim().is_empty() {
        bail!("No Azure client ID configured. Add it in Settings first.");
    }
    let auth_url = AuthUrl::new(format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
        cfg.tenant
    ))?;
    let token_url = TokenUrl::new(format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        cfg.tenant
    ))?;

    let mut client = MsClient::new(ClientId::new(cfg.client_id.clone()), None, auth_url, Some(token_url))
        // Public clients send the client_id in the request body (no secret).
        .set_auth_type(AuthType::RequestBody);

    if let Some(r) = redirect {
        client = client.set_redirect_uri(RedirectUrl::new(r)?);
    }
    Ok(client)
}

/// Result of an interactive login.
pub struct LoginResult {
    pub access_token: String,
    pub email: Option<String>,
}

/// Runs the full interactive login: opens the browser, captures the redirect on
/// a local loopback server, exchanges the code, and persists the refresh token.
///
/// `open_browser` is a callback so the caller controls *how* the URL is opened
/// (e.g. via the Tauri opener plugin).
pub fn interactive_login(
    cfg: &mut AppConfig,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<LoginResult> {
    // Bind loopback on a random free port to receive the auth code.
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| anyhow!("failed to start loopback server: {e}"))?;
    let port = server.server_addr().to_ip().context("loopback addr")?.port();
    let redirect = format!("http://localhost:{port}");

    let client = build_client(cfg, Some(redirect))?;

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut auth = client.authorize_url(CsrfToken::new_random);
    for s in SCOPES {
        auth = auth.add_scope(Scope::new((*s).to_string()));
    }
    let (auth_url, csrf) = auth.set_pkce_challenge(pkce_challenge).url();

    open_browser(auth_url.as_str()).context("opening browser for sign-in")?;

    // Wait for the redirect (with a generous timeout so a stalled login can't hang forever).
    let (code, state) = wait_for_code(&server, Duration::from_secs(300))?;
    if state != *csrf.secret() {
        bail!("OAuth state mismatch — possible CSRF, aborting.");
    }

    let token = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request(oauth2::reqwest::http_client)
        .map_err(|e| anyhow!("token exchange failed: {e}"))?;

    let access_token = token.access_token().secret().clone();
    let refresh_token = token
        .refresh_token()
        .map(|r| r.secret().clone())
        .ok_or_else(|| anyhow!("no refresh token returned (is offline_access granted?)"))?;

    // Learn the email from the id_token (best-effort but normally present).
    let email = token
        .extra_fields()
        .id_token
        .as_deref()
        .and_then(email_from_id_token);
    if email.is_some() {
        cfg.email = email.clone();
    }

    cfg.save()?;
    config::store_refresh_token(cfg, &refresh_token)?;

    Ok(LoginResult { access_token, email })
}

/// Exchanges the stored refresh token for a fresh access token. Rotates and
/// re-stores the refresh token if Microsoft returns a new one.
pub fn refresh_access_token(cfg: &AppConfig) -> Result<String> {
    let refresh = config::load_refresh_token(cfg)?
        .ok_or_else(|| anyhow!("not signed in (no refresh token)"))?;
    let client = build_client(cfg, None)?;

    let token = client
        .exchange_refresh_token(&RefreshToken::new(refresh))
        .add_scopes(SCOPES.iter().map(|s| Scope::new((*s).to_string())))
        .request(oauth2::reqwest::http_client)
        .map_err(|e| anyhow!("refresh failed: {e}"))?;

    if let Some(new_refresh) = token.refresh_token() {
        config::store_refresh_token(cfg, new_refresh.secret())?;
    }
    Ok(token.access_token().secret().clone())
}

/// Blocks until the loopback server receives a request carrying `?code=...`.
fn wait_for_code(server: &tiny_http::Server, timeout: Duration) -> Result<(String, String)> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| anyhow!("timed out waiting for sign-in"))?;

        let request = match server.recv_timeout(remaining)? {
            Some(r) => r,
            None => bail!("timed out waiting for sign-in"),
        };

        let url = request.url().to_string();
        let parsed =
            url::Url::parse(&format!("http://localhost{url}")).context("parsing redirect URL")?;

        let mut code = None;
        let mut state = None;
        let mut error = None;
        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.to_string()),
                "state" => state = Some(v.to_string()),
                "error_description" => error = Some(v.to_string()),
                "error" if error.is_none() => error = Some(v.to_string()),
                _ => {}
            }
        }

        let body = if code.is_some() {
            "<html><body style='font-family:sans-serif'><h2>Signed in \u{2714}</h2>\
             <p>You can close this tab and return to Outlook Saver.</p></body></html>"
        } else {
            "<html><body style='font-family:sans-serif'><h2>Sign-in failed</h2>\
             <p>You can close this tab.</p></body></html>"
        };
        let response = tiny_http::Response::from_string(body)
            .with_header("Content-Type: text/html".parse::<tiny_http::Header>().unwrap());
        let _ = request.respond(response);

        if let Some(err) = error {
            bail!("authorization error: {err}");
        }
        if let (Some(c), Some(s)) = (code, state) {
            return Ok((c, s));
        }
        // Ignore unrelated requests (e.g. favicon) and keep waiting.
    }
}

/// Decodes the JWT id_token payload (no signature verification — only used to
/// display the signed-in email) and pulls out a username/email claim.
fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("preferred_username")
        .or_else(|| json.get("email"))
        .or_else(|| json.get("upn"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
