//! New versions (added 2026-10-14). At most once a day, and only while the
//! "Check for new versions" setting is on, Tote reads
//! https://www.codewithlex.site/tote/latest.json. Nothing about her files is
//! sent; the request carries only the app version. An update is installed
//! only after its signature checks out against the public key in
//! tauri.conf.json, and only when she clicks Update.
//!
//! A dry run (TOTE_ROOTS) never goes online, except when the native check
//! sets TOTE_UPDATE_VERIFY: then Tote treats the published release as new,
//! downloads it, verifies its signature, and stops before installing.

use crate::db;
use crate::error::{ErrorCode, Result, ToteError};
use crate::paths;
use crate::state::Core;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Seconds between two checks.
pub const CHECK_EVERY: i64 = 24 * 60 * 60;
const LAST_CHECK_KEY: &str = "update_last_check";
const AVAILABLE_KEY: &str = "update_available";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// True when a check should go online now.
pub fn due(enabled: bool, last_check: Option<i64>, now: i64) -> bool {
    enabled && last_check.is_none_or(|last| now - last >= CHECK_EVERY || now < last)
}

/// A stored update is still worth showing only while she runs another version.
pub fn still_pending(stored: Option<UpdateInfo>, running: &str) -> Option<UpdateInfo> {
    stored.filter(|info| info.version != running)
}

fn verify_run() -> bool {
    paths::Roots::test_mode() && std::env::var_os("TOTE_UPDATE_VERIFY").is_some()
}

fn network(error: impl std::fmt::Display) -> ToteError {
    ToteError::new(ErrorCode::Network, error.to_string())
}

fn stored(core: &Core) -> Option<UpdateInfo> {
    let conn = core.db().ok()?;
    let json = db::get_setting(&conn, AVAILABLE_KEY).ok()??;
    serde_json::from_str(&json).ok()
}

fn remember(core: &Core, now: i64, found: Option<&UpdateInfo>) -> Result<()> {
    let conn = core.db()?;
    db::set_setting(&conn, LAST_CHECK_KEY, &now.to_string())?;
    let json = match found {
        Some(info) => serde_json::to_string(info).map_err(|e| ToteError::validation(e.to_string()))?,
        None => String::new(),
    };
    db::set_setting(&conn, AVAILABLE_KEY, &json)
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater> {
    let mut builder = app.updater_builder().timeout(std::time::Duration::from_secs(20));
    if verify_run() {
        builder = builder.version_comparator(|_, _| true);
    }
    builder.build().map_err(network)
}

/// The update to offer her, if any. Goes online only when a check is due;
/// otherwise answers from what the last check found.
pub async fn check(app: &AppHandle, core: &Core, enabled: bool) -> Result<Option<UpdateInfo>> {
    let running = app.package_info().version.to_string();
    if paths::Roots::test_mode() && !verify_run() {
        return Ok(None);
    }
    let now = chrono::Utc::now().timestamp();
    let last = {
        let conn = core.db()?;
        db::get_setting(&conn, LAST_CHECK_KEY)?.and_then(|v| v.parse::<i64>().ok())
    };
    if !verify_run() && !due(enabled, last, now) {
        return Ok(if enabled { still_pending(stored(core), &running) } else { None });
    }
    let found = updater(app)?.check().await.map_err(network)?.map(|update| UpdateInfo { version: update.version, notes: update.body });
    remember(core, now, found.as_ref())?;
    match &found {
        Some(info) => log::info!("update: version {} is available (running {running})", info.version),
        None => log::info!("update: running {running}, nothing newer"),
    }
    Ok(found)
}

/// Downloads the newest release, verifies its signature and installs it.
/// On Windows the installer closes Tote and opens the new version.
pub async fn install(app: &AppHandle) -> Result<()> {
    let update = updater(app)?
        .check()
        .await
        .map_err(network)?
        .ok_or_else(|| ToteError::validation("Tote is already up to date."))?;
    let version = update.version.clone();
    let bytes = update.download(|_, _| {}, || {}).await.map_err(network)?;
    log::info!("update: {version} downloaded ({} bytes) and its signature verified", bytes.len());
    if verify_run() {
        log::info!("update: dry run, so {version} is not installed");
        return Ok(());
    }
    update.install(bytes).map_err(network)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn checks_at_most_once_a_day() {
        let now = 1_800_000_000;
        assert!(due(true, None, now), "first start checks");
        assert!(!due(true, Some(now - 3600), now), "an hour later does not");
        assert!(due(true, Some(now - CHECK_EVERY), now), "a day later does");
        assert!(due(true, Some(now + 3600), now), "a clock set back does not block checks forever");
    }

    #[test]
    fn the_setting_turns_checks_off() {
        assert!(!due(false, None, 1_800_000_000));
    }

    #[test]
    fn a_stored_update_disappears_once_installed() {
        let info = UpdateInfo { version: "0.3.0".into(), notes: None };
        assert_eq!(still_pending(Some(info.clone()), "0.2.0"), Some(info.clone()));
        assert_eq!(still_pending(Some(info), "0.3.0"), None);
        assert_eq!(still_pending(None, "0.2.0"), None);
    }
}
