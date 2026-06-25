//! IMAP watcher: connects to Outlook over IMAP using an OAuth2 access token
//! (XOAUTH2), then polls the INBOX for newly arrived messages and hands each
//! one to the saver.
//!
//! Polling (vs. IMAP IDLE) is used deliberately: it is trivially cancellable via
//! the stop flag and reconnects cleanly. The interval is short enough that new
//! mail is saved within a few seconds of arrival.

use anyhow::{anyhow, bail, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::auth;
use crate::config::AppConfig;
use crate::saver;

const IMAP_HOST: &str = "outlook.office365.com";
const IMAP_PORT: u16 = 993;
const POLL_INTERVAL: Duration = Duration::from_secs(15);
const RECONNECT_DELAY: Duration = Duration::from_secs(20);

/// XOAUTH2 SASL authenticator for the `imap` crate.
struct XOAuth2 {
    user: String,
    access_token: String,
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> Self::Response {
        // The imap crate base64-encodes this response for us.
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}

fn emit_status(app: &AppHandle, connected: bool, message: &str) {
    let _ = app.emit(
        "watcher-status",
        serde_json::json!({ "connected": connected, "message": message }),
    );
}

fn emit_log(app: &AppHandle, message: &str) {
    let _ = app.emit("log", serde_json::json!({ "message": message }));
}

/// Runs the watcher until `stop` is set. Reconnects on errors. Intended to run
/// on a dedicated worker thread.
pub fn run(app: AppHandle, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match connect_and_watch(&app, &stop) {
            Ok(()) => {
                // Clean stop requested.
            }
            Err(e) => {
                emit_status(&app, false, &format!("Disconnected: {e}"));
                emit_log(&app, &format!("Error: {e:#}"));
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Wait before reconnecting, but stay responsive to stop.
        sleep_cancellable(RECONNECT_DELAY, &stop);
    }
    emit_status(&app, false, "Stopped");
}

fn connect_and_watch(app: &AppHandle, stop: &Arc<AtomicBool>) -> Result<()> {
    let cfg = AppConfig::load()?;
    let email = cfg
        .email
        .clone()
        .ok_or_else(|| anyhow!("no signed-in account; please sign in again"))?;

    emit_status(app, false, "Refreshing access token\u{2026}");
    let access_token = auth::refresh_access_token(&cfg)?;

    emit_status(app, false, &format!("Connecting to {IMAP_HOST}\u{2026}"));
    let tls = native_tls::TlsConnector::new().context("building TLS connector")?;
    let client = imap::connect((IMAP_HOST, IMAP_PORT), IMAP_HOST, &tls)
        .context("connecting to IMAP server")?;

    let auth = XOAuth2 {
        user: email.clone(),
        access_token,
    };
    let mut session = client
        .authenticate("XOAUTH2", &auth)
        .map_err(|(e, _client)| anyhow!("XOAUTH2 authentication failed: {e}"))?;

    let mailbox = session.select("INBOX").context("selecting INBOX")?;
    // Messages arriving from now on will have a UID >= uid_next at connect time.
    let mut next_uid = mailbox.uid_next.unwrap_or(1);

    emit_status(
        app,
        true,
        &format!("Watching {email} — new mail will be saved as it arrives"),
    );
    emit_log(app, &format!("Connected. Watching for UID \u{2265} {next_uid}"));

    while !stop.load(Ordering::Relaxed) {
        // Search for anything at or beyond our cursor. `n:*` always returns the
        // highest message, so we filter strictly by UID.
        let uids = session
            .uid_search(format!("UID {next_uid}:*"))
            .context("searching for new mail")?;

        let mut fresh: Vec<u32> = uids.into_iter().filter(|&u| u >= next_uid).collect();
        fresh.sort_unstable();

        for uid in fresh {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let fetched = session
                .uid_fetch(uid.to_string(), "BODY[]")
                .with_context(|| format!("fetching UID {uid}"))?;

            for msg in fetched.iter() {
                let Some(body) = msg.body() else { continue };
                let name = saver::suggest_filename(body);
                // Reload config so destination/prompt prefs reflect latest UI changes.
                let cfg = AppConfig::load().unwrap_or_else(|_| cfg.clone());
                match saver::save_eml(app, &cfg, &name, body) {
                    Ok(Some(path)) => {
                        emit_log(app, &format!("Saved: {}", path.display()));
                        let _ = app.emit(
                            "mail-saved",
                            serde_json::json!({ "path": path.to_string_lossy(), "name": name }),
                        );
                    }
                    Ok(None) => emit_log(app, &format!("Skipped (cancelled): {name}")),
                    Err(e) => emit_log(app, &format!("Failed to save {name}: {e:#}")),
                }
            }
            next_uid = uid + 1;
        }

        sleep_cancellable(POLL_INTERVAL, stop);
    }

    let _ = session.logout();
    Ok(())
}

/// Sleeps up to `dur` but wakes early (and frequently) to honor the stop flag.
fn sleep_cancellable(dur: Duration, stop: &Arc<AtomicBool>) {
    let step = Duration::from_millis(250);
    let mut elapsed = Duration::ZERO;
    while elapsed < dur {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(step);
        elapsed += step;
    }
}

/// Validates that an account is signed in and a client id is set.
pub fn preflight() -> Result<()> {
    let cfg = AppConfig::load()?;
    if cfg.client_id.trim().is_empty() {
        bail!("Set your Azure client ID in Settings first.");
    }
    if cfg.email.is_none() {
        bail!("Sign in first.");
    }
    Ok(())
}
