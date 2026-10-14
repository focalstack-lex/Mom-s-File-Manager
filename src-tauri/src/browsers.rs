//! Browser download settings (spec 5.1.2). Tote only files what lands in
//! Downloads, so a browser that asks where to save, or saves somewhere else,
//! means missed downloads. This module reads each browser's own settings
//! file (read only, never written) and says which ones need a change.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub browser: String,
    pub profile: String,
    /// True when downloads go straight to the Downloads folder.
    pub ok: bool,
    /// The browser asks where to save each file.
    pub asks: bool,
    /// Where downloads go when that is not Downloads.
    pub folder: Option<String>,
    /// One plain sentence: what is wrong and where to change it.
    pub advice: String,
}

/// Chrome, Edge and Brave share the Chromium `Preferences` JSON format.
pub fn chromium_status(browser: &str, prefs_json: &str, downloads: &Path) -> Option<BrowserStatus> {
    let value: serde_json::Value = serde_json::from_str(prefs_json).ok()?;
    let profile = value["profile"]["name"].as_str().unwrap_or("Default").to_string();
    let asks = value["download"]["prompt_for_download"].as_bool().unwrap_or(false);
    let folder = value["download"]["default_directory"].as_str().filter(|d| !d.trim().is_empty()).map(str::to_string);
    let elsewhere = folder.as_deref().filter(|d| !same_folder(Path::new(d), downloads)).map(str::to_string);
    Some(verdict(browser, profile, asks, elsewhere, "Settings > Downloads"))
}

/// Firefox keeps its settings as `user_pref("name", value);` lines in prefs.js.
pub fn firefox_status(profile: &str, prefs_js: &str, downloads: &Path, desktop: Option<&Path>) -> BrowserStatus {
    let mut use_download_dir = true;
    let mut folder_list = 1_i64;
    let mut dir: Option<String> = None;
    for line in prefs_js.lines() {
        let Some((name, value)) = parse_user_pref(line) else { continue };
        match name.as_str() {
            "browser.download.useDownloadDir" => use_download_dir = value != "false",
            "browser.download.folderList" => folder_list = value.parse().unwrap_or(1),
            "browser.download.dir" => dir = Some(value.trim_matches('"').replace("\\\\", "\\")),
            _ => {}
        }
    }
    let target = match folder_list {
        0 => desktop.map(|d| d.to_string_lossy().into_owned()),
        2 => dir,
        _ => None,
    };
    let elsewhere = target.filter(|d| !same_folder(Path::new(d), downloads));
    verdict("Firefox", profile.to_string(), !use_download_dir, elsewhere, "Settings > General > Files and Applications")
}

fn verdict(browser: &str, profile: String, asks: bool, elsewhere: Option<String>, settings_path: &str) -> BrowserStatus {
    let advice = match (asks, elsewhere.as_deref()) {
        (true, _) => format!("Asks where to save each file. In {browser}, open {settings_path} and turn that off, with the location set to Downloads."),
        (false, Some(dir)) => format!("Saves to {dir}. In {browser}, open {settings_path} and change the location to Downloads."),
        (false, None) => "Saves to Downloads. Nothing to change.".to_string(),
    };
    BrowserStatus { browser: browser.to_string(), profile, ok: !asks && elsewhere.is_none(), asks, folder: elsewhere, advice }
}

/// `user_pref("browser.download.dir", "C:\\Users\\x");` becomes (name, raw value).
fn parse_user_pref(line: &str) -> Option<(String, String)> {
    let inner = line.trim().strip_prefix("user_pref(")?.strip_suffix(");")?;
    let (name, value) = inner.split_once(',')?;
    Some((name.trim().trim_matches('"').to_string(), value.trim().to_string()))
}

fn same_folder(a: &Path, b: &Path) -> bool {
    crate::paths::same_path(a, b)
}

/// Every installed browser profile Tote can read, in a stable order.
pub fn check(downloads: &Path, desktop: Option<&Path>) -> Vec<BrowserStatus> {
    let mut out = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        for (browser, rel) in [
            ("Chrome", "Google\\Chrome\\User Data"),
            ("Edge", "Microsoft\\Edge\\User Data"),
            ("Brave", "BraveSoftware\\Brave-Browser\\User Data"),
        ] {
            for prefs in chromium_profiles(&local.join(rel)) {
                match std::fs::read_to_string(&prefs) {
                    Ok(text) => {
                        if let Some(status) = chromium_status(browser, &text, downloads) {
                            out.push(status);
                        }
                    }
                    Err(error) => log::debug!("browser prefs {}: {error}", prefs.display()),
                }
            }
        }
    }
    if let Some(roaming) = dirs::data_dir() {
        let profiles = roaming.join("Mozilla\\Firefox\\Profiles");
        if let Ok(read) = std::fs::read_dir(&profiles) {
            let mut dirs: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.join("prefs.js").is_file()).collect();
            dirs.sort();
            for dir in dirs {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let label = name.split_once('.').map(|(_, rest)| rest.to_string()).unwrap_or(name);
                if let Ok(text) = std::fs::read_to_string(dir.join("prefs.js")) {
                    out.push(firefox_status(&label, &text, downloads, desktop));
                }
            }
        }
    }
    out
}

/// `Default` and `Profile N` folders that hold a Preferences file.
fn chromium_profiles(user_data: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(user_data) else { return Vec::new() };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name == "Default" || name.starts_with("Profile ")
        })
        .map(|p| p.join("Preferences"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const DOWNLOADS: &str = r"C:\Users\Mom\Downloads";

    #[test]
    fn chromium_default_is_fine() {
        let status = chromium_status("Chrome", r#"{"profile":{"name":"Mom"},"download":{"prompt_for_download":false}}"#, Path::new(DOWNLOADS)).unwrap();
        assert!(status.ok);
        assert_eq!(status.profile, "Mom");
        let explicit = format!(r#"{{"download":{{"default_directory":"{}"}}}}"#, DOWNLOADS.replace('\\', "\\\\").to_uppercase());
        assert!(chromium_status("Edge", &explicit, Path::new(DOWNLOADS)).unwrap().ok);
    }

    #[test]
    fn chromium_asking_or_elsewhere_needs_a_change() {
        let asks = chromium_status("Chrome", r#"{"download":{"prompt_for_download":true}}"#, Path::new(DOWNLOADS)).unwrap();
        assert!(!asks.ok && asks.asks);
        assert!(asks.advice.contains("Asks where to save"));
        let elsewhere = chromium_status("Brave", r#"{"download":{"default_directory":"D:\\Files"}}"#, Path::new(DOWNLOADS)).unwrap();
        assert!(!elsewhere.ok);
        assert_eq!(elsewhere.folder.as_deref(), Some(r"D:\Files"));
        assert!(chromium_status("Chrome", "not json", Path::new(DOWNLOADS)).is_none());
    }

    #[test]
    fn firefox_prefs_are_read() {
        let fine = firefox_status("default-release", "user_pref(\"browser.startup.page\", 3);\n", Path::new(DOWNLOADS), None);
        assert!(fine.ok);
        let asks = firefox_status("p", "user_pref(\"browser.download.useDownloadDir\", false);\n", Path::new(DOWNLOADS), None);
        assert!(asks.asks && !asks.ok);
        let custom = "user_pref(\"browser.download.folderList\", 2);\nuser_pref(\"browser.download.dir\", \"E:\\\\Saved\");\n";
        let elsewhere = firefox_status("p", custom, Path::new(DOWNLOADS), None);
        assert_eq!(elsewhere.folder.as_deref(), Some(r"E:\Saved"));
        let desktop = firefox_status("p", "user_pref(\"browser.download.folderList\", 0);\n", Path::new(DOWNLOADS), Some(Path::new(r"C:\Users\Mom\Desktop")));
        assert_eq!(desktop.folder.as_deref(), Some(r"C:\Users\Mom\Desktop"));
    }
}
