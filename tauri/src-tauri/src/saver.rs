//! Turns a raw RFC822 message into a saved `.eml` file, optionally prompting the
//! user for the destination via the native Save dialog.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::config::AppConfig;

/// Saves `raw` (a full RFC822 message) as a `.eml` file.
///
/// Returns the path written, or `None` if the user cancelled the dialog.
pub fn save_eml(
    app: &AppHandle,
    cfg: &AppConfig,
    suggested_name: &str,
    raw: &[u8],
) -> Result<Option<PathBuf>> {
    // Silent mode: a default folder is set and the user opted out of prompting.
    if !cfg.ask_each_time {
        if let Some(dir) = &cfg.default_save_dir {
            let dir = PathBuf::from(dir);
            fs::create_dir_all(&dir)
                .with_context(|| format!("creating save dir {}", dir.display()))?;
            let path = unique_path(dir.join(suggested_name));
            fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
            return Ok(Some(path));
        }
        // No default dir configured — fall through to prompting.
    }

    // Prompt the user. blocking_save_file() must be called off the main thread,
    // which is the case here (we run inside the watcher worker thread).
    let mut builder = app
        .dialog()
        .file()
        .set_title("Save Outlook email")
        .set_file_name(suggested_name)
        .add_filter("Email", &["eml"]);
    if let Some(dir) = &cfg.default_save_dir {
        builder = builder.set_directory(dir);
    }

    match builder.blocking_save_file() {
        Some(file_path) => {
            let path = file_path
                .into_path()
                .map_err(|e| anyhow!("invalid save path: {e}"))?;
            fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))?;
            Ok(Some(path))
        }
        None => Ok(None), // user cancelled
    }
}

/// Builds a friendly, filesystem-safe filename from the message headers, e.g.
/// `2026-06-24_1530_Quarterly-report.eml`.
pub fn suggest_filename(raw: &[u8]) -> String {
    let (subject, date) = parse_headers(raw);
    let prefix = date.unwrap_or_else(|| "email".to_string());
    let subject = subject.unwrap_or_else(|| "no-subject".to_string());
    let safe_subject = sanitize(&subject);
    let stem = format!("{prefix}_{safe_subject}");
    // Keep filenames within sane length limits.
    let stem: String = stem.chars().take(120).collect();
    format!("{stem}.eml")
}

fn parse_headers(raw: &[u8]) -> (Option<String>, Option<String>) {
    let mut subject = None;
    let mut date = None;
    if let Ok((headers, _)) = mailparse::parse_headers(raw) {
        for h in headers {
            match h.get_key_ref().to_ascii_lowercase().as_str() {
                "subject" => subject = Some(h.get_value()),
                "date" => {
                    // Convert RFC2822 date to a sortable YYYY-MM-DD_HHMM prefix.
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(h.get_value().trim()) {
                        date = Some(dt.format("%Y-%m-%d_%H%M").to_string());
                    }
                }
                _ => {}
            }
        }
    }
    (subject, date)
}

/// Replaces characters that are illegal or awkward in filenames.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "no-subject".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Avoids clobbering an existing file by appending ` (n)` before the extension.
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("email");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("eml");
    let parent = path.parent().map(PathBuf::from).unwrap_or_default();
    for n in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}
