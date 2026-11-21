//! Library paths from the platform: the folder picker, the drop handler,
//! and the one acceptance rule both share.
//!
//! The acceptance and the report wording are the egui crate's
//! (`ui/app/library_picker.rs`): a candidate must exist and be a directory,
//! the canonical form is what registers, a re-drop of a registered root never
//! rescans, and a refusal reads as the standard sentence. A drop indexes
//! immediately; the picker registers and waits for the user's rescan.

use std::path::{Path, PathBuf};

use nsh_core::file_dialog::OpenDialog;
use nsh_core::prelude::*;
use riff_backend::app::MutexExt;

use crate::bridge::UiModel;
use crate::bridge::state::UiState;
use crate::model::SettingsPage;

/// The shared acceptance judgment: the canonical directory, or the standard
/// refusal sentence.
pub fn judge(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }
    Ok(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Register every accepted candidate, deduping against the session, and
/// (for a drop) request each new root's scan. Refusals win the status line,
/// exactly the egui report order.
pub fn accept_paths(model: Stored<UiModel>, state: UiState, candidates: &[PathBuf], scan: bool) {
    let mut registered = Vec::new();
    let mut rejections = Vec::new();
    for candidate in candidates {
        match judge(candidate) {
            Err(refusal) => rejections.push(refusal),
            Ok(canonical) => {
                let known = model.with(|m| {
                    m.rt.library
                        .lock_or_recover()
                        .library_paths
                        .paths()
                        .contains(&canonical)
                });
                if known {
                    continue;
                }
                model.with(|m| {
                    let mut library = m.rt.library.lock_or_recover();
                    library
                        .library_paths
                        .register(canonical.clone(), m.rt.settings.as_mut());
                });
                if scan {
                    model.with(|m| m.rt.scans.request(canonical.clone()));
                }
                registered.push(canonical);
            }
        }
    }
    if !registered.is_empty() {
        let line = if scan {
            "Added library path, scanning\u{2026}".to_owned()
        } else {
            format!(
                "Added library path. Rescan it to index {}.",
                registered[0].display()
            )
        };
        state.action_status.set(Some(line));
        state
            .settings_status
            .set(Some(format!("Added {} folder(s)", registered.len())));
    }
    for refusal in rejections {
        state.action_status.set(Some(refusal.clone()));
        state.settings_status.set(Some(refusal));
    }
}

/// Open the system folder picker. A platform whose dialog errors lands on
/// the typed-path fallback: the Settings Library page, whose field takes the
/// same path.
pub fn pick_folders(model: Stored<UiModel>, state: UiState) {
    spawn(async move {
        let answer = pick_paths(
            OpenDialog::folders()
                .multiple(true)
                .title("Add music folders"),
        )
        .await;
        match answer {
            Ok(Some(paths)) => accept_paths(model, state, &paths, false),
            Ok(None) => {}
            Err(e) => {
                state.action_status.set(Some(format!(
                    "The folder picker could not open ({e}). Type the path in Settings \u{2192} Library."
                )));
                crate::screens::settings::open(model, state);
                state.settings_page.set(SettingsPage::Library);
            }
        }
    });
}

/// Accept one drop's `file:` URIs.
pub fn accept_drop(model: Stored<UiModel>, state: UiState, uris: &[String]) {
    let paths: Vec<PathBuf> = uris.iter().filter_map(|uri| path_from_uri(uri)).collect();
    accept_paths(model, state, &paths, true);
}

/// A `file:` URI as a local path: the scheme stripped, percent escapes
/// decoded, and Windows' leading slash before the drive letter dropped.
#[must_use]
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    #[cfg(windows)]
    let decoded = decoded.strip_prefix('/').unwrap_or(&decoded).to_owned();
    Some(PathBuf::from(decoded))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2]))
        {
            out.push(high * 16 + low);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_decodes_to_a_path() {
        let path = path_from_uri("file:///music/My%20Album").expect("a file uri parses");
        assert!(path.to_string_lossy().contains("My Album"));
        assert!(path_from_uri("https://example.com/x").is_none());
    }
}
