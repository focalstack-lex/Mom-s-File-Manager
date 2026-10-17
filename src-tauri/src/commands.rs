//! The command layer: validates input, calls the modules, and returns plain
//! data. The renderer never touches the filesystem except through here.
//!
//! Every command that touches the disk or the database is `async` and runs
//! its work on the blocking pool. A sync command runs on the main thread,
//! where a shell call (the Recycle Bin, Explorer) can stall the window.

use crate::cleanup::{self, CleanupReport};
use crate::clipboard;
use crate::db;
use crate::error::{ToteError, ErrorCode, Result, FILE_GONE};
use crate::extract;
use crate::index::{self, FileRow, Filters, Group, IndexStatus};
use crate::ops::{self, OpRow};
use crate::paths;
use crate::sorter;
use crate::state::{AppState, Core};
use crate::tasks::{self, Completed, Repeat, Task};
use crate::templates::{self, Template};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub name: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub start_at_login: bool,
    pub notifications: bool,
    pub tour_done: bool,
    pub school_year: Option<DateRange>,
    pub quarters: Vec<DateRange>,
    /// Folders she added in Settings (spec 5.8).
    pub extra_folders: Vec<String>,
    /// Default folders she switched off (Desktop or OneDrive).
    pub excluded_folders: Vec<String>,
    /// Her subject words for Organize; empty means the standard DepEd list.
    pub subject_rules: Vec<crate::organize::SubjectRule>,
    /// Once a day, ask codewithlex.site whether a newer Tote exists.
    pub check_updates: bool,
    /// She has opened To do once; the sidebar's New marker goes.
    pub todo_seen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_at_login: true,
            notifications: true,
            tour_done: false,
            school_year: None,
            quarters: Vec::new(),
            extra_folders: Vec::new(),
            excluded_folders: Vec::new(),
            subject_rules: Vec::new(),
            check_updates: true,
            todo_seen: false,
        }
    }
}

/// The subject rules in effect. Takes its own short lock (via settings_of),
/// so call it before taking `core.db()`.
fn rules_of(core: &Core) -> Vec<crate::organize::SubjectRule> {
    crate::organize::effective_rules(&settings_of(core).subject_rules)
}

/// The stored settings, or the defaults when nothing is stored. The text is
/// read one field at a time (`read_settings`), so a field a hand edit or
/// another Tote version wrote in a different shape resets only itself; when
/// any field was reset the text is logged and kept aside under
/// `settings.unreadable-<time>` (once per text), so the next save never
/// loses what she had (audit T-47). The live row is left as it is for a
/// newer Tote to read.
pub fn settings_from(conn: &rusqlite::Connection) -> Settings {
    match db::get_setting(conn, "settings") {
        Ok(Some(json)) => {
            let (settings, reset) = read_settings(&json);
            if !reset.is_empty() {
                log::warn!("settings could not be read in full; reset to the defaults: {}", reset.join(", "));
                keep_unreadable(conn, &json);
            }
            settings
        }
        Ok(None) => Settings::default(),
        Err(error) => {
            log::warn!("settings could not be loaded: {}", error.message);
            Settings::default()
        }
    }
}

/// Reads settings text one field at a time. A missing field is its default
/// (an older Tote never wrote it); a field that cannot be read is its
/// default and is named in the list, with the reason logged. Text that is
/// not a JSON object at all resets everything and names "settings".
fn read_settings(json: &str) -> (Settings, Vec<&'static str>) {
    let object = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(object)) => object,
        Ok(_) => {
            log::warn!("settings text is not an object; using the defaults");
            return (Settings::default(), vec!["settings"]);
        }
        Err(error) => {
            log::warn!("settings text could not be read ({error}); using the defaults");
            return (Settings::default(), vec!["settings"]);
        }
    };
    let mut reset = Vec::new();
    let defaults = Settings::default();
    let settings = Settings {
        start_at_login: field(&object, "startAtLogin", defaults.start_at_login, &mut reset),
        notifications: field(&object, "notifications", defaults.notifications, &mut reset),
        tour_done: field(&object, "tourDone", defaults.tour_done, &mut reset),
        school_year: field(&object, "schoolYear", defaults.school_year, &mut reset),
        quarters: field(&object, "quarters", defaults.quarters, &mut reset),
        extra_folders: field(&object, "extraFolders", defaults.extra_folders, &mut reset),
        excluded_folders: field(&object, "excludedFolders", defaults.excluded_folders, &mut reset),
        subject_rules: field(&object, "subjectRules", defaults.subject_rules, &mut reset),
        check_updates: field(&object, "checkUpdates", defaults.check_updates, &mut reset),
        todo_seen: field(&object, "todoSeen", defaults.todo_seen, &mut reset),
    };
    (settings, reset)
}

/// One field of the settings text: its value when it reads, else `default`
/// with the field's name added to `reset`.
fn field<T: serde::de::DeserializeOwned>(object: &serde_json::Map<String, serde_json::Value>, name: &'static str, default: T, reset: &mut Vec<&'static str>) -> T {
    let Some(value) = object.get(name) else { return default };
    match serde_json::from_value(value.clone()) {
        Ok(read) => read,
        Err(error) => {
            log::warn!("settings field {name} could not be read ({error}); its default is used");
            reset.push(name);
            default
        }
    }
}

/// Stores unreadable settings text beside the live row, once per text.
fn keep_unreadable(conn: &rusqlite::Connection, json: &str) {
    let kept = conn.query_row("SELECT 1 FROM settings WHERE key LIKE 'settings.unreadable-%' AND value = ?1", [json], |_| Ok(())).optional();
    match kept {
        Ok(Some(())) => {}
        Ok(None) => {
            let key = format!("settings.unreadable-{}", db::now());
            match db::set_setting(conn, &key, json) {
                Ok(()) => log::warn!("the unreadable settings text is kept as {key}"),
                Err(error) => log::warn!("the unreadable settings text could not be kept: {}", error.message),
            }
        }
        Err(error) => log::warn!("could not look for kept settings text: {error}"),
    }
}

/// Reads settings with its own short lock. Never call it while a `core.db()`
/// guard is alive: the mutex is not reentrant.
pub fn settings_of(core: &Core) -> Settings {
    let Ok(conn) = core.db() else { return Settings::default() };
    settings_from(&conn)
}

/// The root choices stored in Settings, read before the roots are built.
pub fn root_choices(conn: &rusqlite::Connection) -> paths::RootChoices {
    let settings = settings_from(conn);
    paths::RootChoices {
        extra: settings.extra_folders.iter().map(PathBuf::from).collect(),
        excluded: settings.excluded_folders.iter().map(PathBuf::from).collect(),
    }
}

fn save_settings(core: &Core, settings: &Settings) -> Result<()> {
    let json = serde_json::to_string(settings).map_err(ToteError::db_write)?;
    let conn = core.db()?;
    db::set_setting(&conn, "settings", &json).map_err(ToteError::as_write)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub roots: Vec<String>,
    /// Roots that Settings may switch off (Documents and Downloads stay).
    pub removable_roots: Vec<String>,
    pub test_mode: bool,
    pub school_files: String,
    pub downloads: Option<String>,
    pub settings: Settings,
    pub index: IndexStatus,
    pub sorted_this_week: usize,
    pub recovered: bool,
    /// tote.sqlite was rebuilt on this start; `kept` says whether her
    /// settings, pins, templates and Activity came along.
    pub rebuilt: Option<db::Rebuilt>,
    /// A damaged to-do list was replaced on this start.
    pub tasks_restored: Option<tasks::Restored>,
    /// The To do store could not be opened on this start.
    pub tasks_unavailable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    pub groups: Vec<Group>,
    pub pinned: Vec<FileRow>,
    pub orphan_pins: Vec<String>,
    pub recent: Vec<FileRow>,
    pub index: IndexStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: String,
    pub text: Option<String>,
    pub name: String,
    pub size: u64,
    pub modified: i64,
    pub parent: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub path: String,
    pub label: String,
    pub recent: bool,
}

/// Runs `work` on the blocking pool and logs any failure, so a problem she
/// sees as a toast is also in the log file.
async fn blocking<T: Send + 'static>(name: &'static str, work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    let joined = tauri::async_runtime::spawn_blocking(work).await.map_err(|error| {
        log::warn!("{name}: the task did not finish ({error})");
        ToteError::new(ErrorCode::Io, "Tote could not finish that. Try again in a moment.")
    });
    log_failure(name, joined.and_then(|r| r))
}

/// Logs a failed command under its name; the commands that stay on the
/// async runtime (the update check and install) use it directly. The Warn
/// line names the kind of file, never its name, which often carries a
/// student's; the path is at Debug (audit T-52).
fn log_failure<T>(name: &str, result: Result<T>) -> Result<T> {
    if let Err(error) = &result {
        match &error.path {
            Some(path) => {
                let what = if Path::new(path).is_dir() { "a folder" } else { sorter::describe(Path::new(path)) };
                log::warn!("{name}: {} ({:?}) for {what}", error.message, error.code);
                log::debug!("{name}: the path is {path}");
            }
            None => log::warn!("{name}: {} ({:?})", error.message, error.code),
        }
    }
    result
}

/// An opener plugin call that failed: the raw error is logged, she reads `sentence`.
fn opener_failed(what: &'static str, sentence: &'static str) -> impl FnOnce(tauri_plugin_opener::Error) -> ToteError {
    move |error| {
        log::warn!("{what}: {error}");
        ToteError::new(ErrorCode::Io, sentence)
    }
}

fn core_of(state: &State<'_, AppState>) -> Arc<Core> {
    state.0.clone()
}

#[tauri::command]
pub async fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    let core = core_of(&state);
    blocking("get_app_info", move || {
        // Settings first; settings_of takes its own lock.
        let settings = settings_of(&core);
        let conn = core.db()?;
        let week_ago = db::now() - 7 * 86_400;
        Ok(AppInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            roots: core.roots.all().iter().map(|p: &PathBuf| paths::display(p)).collect(),
            removable_roots: core.roots.all().iter().filter(|p| core.roots.removable(p)).map(|p: &PathBuf| paths::display(p)).collect(),
            test_mode: paths::Roots::test_mode(),
            school_files: core.roots.school_files().map(|p: PathBuf| paths::display(&p)).unwrap_or_default(),
            downloads: core.roots.downloads.as_deref().map(|p: &Path| paths::display(p)),
            settings,
            index: core.status(),
            sorted_this_week: index::sorted_since(&conn, week_ago)?,
            recovered: core.recovered,
            rebuilt: core.rebuilt,
            tasks_restored: core.tasks_restored,
            tasks_unavailable: core.tasks_unavailable,
        })
    })
    .await
}

#[tauri::command]
pub async fn find(state: State<'_, AppState>, query: String, filters: Filters) -> Result<FindResult> {
    let core = core_of(&state);
    blocking("find", move || {
        let conn = core.db()?;
        let has_filters = filters.from.is_some() || filters.to.is_some() || !filters.kinds.is_empty() || !filters.sources.is_empty();
        // Punctuation alone is no word, so it is no search: the pinned and recent lists stay.
        let searching = index::fts_query(&query).is_some() || has_filters;
        let groups = if searching { index::group(index::search(&conn, &query, &filters, 200)?) } else { Vec::new() };
        Ok(FindResult {
            groups,
            pinned: if searching { Vec::new() } else { index::pinned(&conn)? },
            orphan_pins: if searching { Vec::new() } else { index::orphan_pins(&conn)? },
            recent: if searching { Vec::new() } else { index::recent_downloads(&conn, 30, 100)? },
            index: core.status(),
        })
    })
    .await
}

#[tauri::command]
pub async fn set_pinned(state: State<'_, AppState>, path: String, pinned: bool) -> Result<()> {
    let core = core_of(&state);
    blocking("set_pinned", move || {
        let confined = core.roots.confine(Path::new(&path))?;
        let conn = core.db()?;
        index::set_pinned(&conn, &paths::display(&confined), pinned)
    })
    .await
}

#[tauri::command]
pub async fn forget_pin(state: State<'_, AppState>, path: String) -> Result<()> {
    let core = core_of(&state);
    blocking("forget_pin", move || {
        let conn = core.db()?;
        index::set_pinned(&conn, &path, false)
    })
    .await
}

#[tauri::command]
pub async fn open_path(state: State<'_, AppState>, app: tauri::AppHandle, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let core = core_of(&state);
    blocking("open_path", move || {
        let confined = core.roots.confine(Path::new(&path))?;
        if !confined.exists() {
            return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(path));
        }
        app.opener()
            .open_path(paths::display(&confined), None::<&str>)
            .map_err(opener_failed("open_path", "Windows could not open that file. Open it from File Explorer instead."))
    })
    .await
}

#[tauri::command]
pub async fn reveal_path(state: State<'_, AppState>, app: tauri::AppHandle, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let core = core_of(&state);
    blocking("reveal_path", move || {
        let confined = core.roots.confine(Path::new(&path))?;
        if !confined.exists() {
            return Err(ToteError::new(ErrorCode::NotFound, FILE_GONE).with_path(path));
        }
        app.opener()
            .reveal_item_in_dir(&confined)
            .map_err(opener_failed("reveal_path", "Windows could not show that file in its folder. Open the folder from File Explorer instead."))
    })
    .await
}

#[tauri::command]
pub async fn open_school_files(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let core = core_of(&state);
    blocking("open_school_files", move || {
        let school = core.roots.school_files()?;
        std::fs::create_dir_all(&school)?;
        app.opener()
            .open_path(paths::display(&school), None::<&str>)
            .map_err(opener_failed("open_school_files", "Windows could not open School Files. Open it from File Explorer instead."))
    })
    .await
}

#[tauri::command]
pub async fn copy_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<()> {
    let core = core_of(&state);
    blocking("copy_files", move || {
        let confined: Vec<PathBuf> = paths.iter().map(|p| core.roots.confine(Path::new(p))).collect::<Result<_>>()?;
        clipboard::check_exists(&confined)?;
        clipboard::copy_files(&confined)
    })
    .await
}

#[tauri::command]
pub async fn preview(state: State<'_, AppState>, path: String) -> Result<Preview> {
    let core = core_of(&state);
    blocking("preview", move || {
        let confined = core.roots.confine(Path::new(&path))?;
        let meta = std::fs::metadata(&confined)?;
        let ext = sorter::extension(&confined);
        let kind = sorter::classify(&ext);
        let name = confined.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = confined.parent().map(paths::display).unwrap_or_default();
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (kind_text, text) = if kind == sorter::Kind::Images {
            ("image".to_string(), None)
        } else {
            // The first lines with their breaks, not one run of text (audit T-46).
            match extract::extract_preview(&confined, &ext, meta.len()) {
                Some(t) => ("text".to_string(), Some(t)),
                None => ("details".to_string(), None),
            }
        };
        Ok(Preview { kind: kind_text, text, name, size: meta.len(), modified, parent })
    })
    .await
}

#[tauri::command]
pub async fn move_files(state: State<'_, AppState>, paths: Vec<String>, destination: String) -> Result<Vec<String>> {
    let core = core_of(&state);
    blocking("move_files", move || {
        let conn = core.db()?;
        let dest = Path::new(&destination);
        // Several files moved together are one batch: one Activity row, one Undo all.
        let batch = (paths.len() > 1).then(|| ops::new_batch("move"));
        let mut moved = Vec::new();
        for p in paths {
            let new_path = match &batch {
                Some(batch) => ops::move_file_in_batch(&conn, &core.roots, &core.placed, Path::new(&p), dest, batch)?,
                None => ops::move_file(&conn, &core.roots, &core.placed, Path::new(&p), dest, ops::OpKind::Move)?,
            };
            moved.push(paths::display(&new_path));
        }
        Ok(moved)
    })
    .await
}

#[tauri::command]
pub async fn rename_file(state: State<'_, AppState>, path: String, new_stem: String) -> Result<String> {
    let core = core_of(&state);
    blocking("rename_file", move || {
        let conn = core.db()?;
        Ok(paths::display(&ops::rename_file(&conn, &core.roots, Path::new(&path), &new_stem)?))
    })
    .await
}

#[tauri::command]
pub async fn create_folder(state: State<'_, AppState>, parent: String, name: String) -> Result<String> {
    let core = core_of(&state);
    blocking("create_folder", move || {
        let conn = core.db()?;
        Ok(paths::display(&ops::create_folder(&conn, &core.roots, Path::new(&parent), &name)?))
    })
    .await
}

#[tauri::command]
pub async fn recycle_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<OpRow>> {
    let core = core_of(&state);
    blocking("recycle_files", move || {
        let conn = core.db()?;
        let targets: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        ops::recycle(&conn, &core.roots, &targets)
    })
    .await
}

/// Folders she can move things into: School Files subfolders first, recent
/// destinations marked, then the top-level folders inside each root.
#[tauri::command]
pub async fn list_destinations(state: State<'_, AppState>) -> Result<Vec<Destination>> {
    let core = core_of(&state);
    blocking("list_destinations", move || {
        let conn = core.db()?;
        let recent: Vec<String> = {
            let mut stmt = conn.prepare("SELECT DISTINCT dst FROM ops WHERE kind = 'move' AND status = 'done' AND dst IS NOT NULL ORDER BY id DESC LIMIT 20")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).filter_map(|p| Path::new(&p).parent().map(paths::display)).collect()
        };
        let mut out: Vec<Destination> = Vec::new();
        let push = |path: PathBuf, label: String, out: &mut Vec<Destination>| {
            let text = paths::display(&path);
            if out.iter().any(|d| d.path == text) {
                return;
            }
            let is_recent = recent.contains(&text);
            out.push(Destination { path: text, label, recent: is_recent });
        };
        let school = core.roots.school_files()?;
        push(school.clone(), "School Files".into(), &mut out);
        if let Ok(read) = std::fs::read_dir(&school) {
            let mut kinds: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            kinds.sort();
            for kind_dir in kinds {
                let kind_name = kind_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                push(kind_dir.clone(), format!("School Files > {kind_name}"), &mut out);
                if let Ok(months) = std::fs::read_dir(&kind_dir) {
                    let mut months: Vec<PathBuf> = months.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                    months.sort();
                    months.reverse();
                    for month in months.into_iter().take(6) {
                        let m = month.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        push(month, format!("School Files > {kind_name} > {m}"), &mut out);
                    }
                }
            }
        }
        for root in core.roots.all() {
            let root_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            push(root.clone(), root_name.clone(), &mut out);
            if let Ok(read) = std::fs::read_dir(&root) {
                let mut dirs: Vec<PathBuf> = read
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .filter(|p| !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true))
                    .filter(|p| p != &school)
                    .collect();
                dirs.sort();
                for d in dirs.into_iter().take(40) {
                    let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    push(d, format!("{root_name} > {name}"), &mut out);
                }
            }
        }
        out.sort_by_key(|d| std::cmp::Reverse(d.recent));
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn list_activity(state: State<'_, AppState>) -> Result<Vec<OpRow>> {
    let core = core_of(&state);
    blocking("list_activity", move || {
        let conn = core.db()?;
        ops::list(&conn, 200)
    })
    .await
}

#[tauri::command]
pub async fn undo(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core_of(&state);
    blocking("undo", move || {
        let conn = core.db()?;
        ops::undo(&conn, &core.roots, &core.placed, id)
    })
    .await
}

#[tauri::command]
pub async fn cleanup_report(state: State<'_, AppState>) -> Result<CleanupReport> {
    let core = core_of(&state);
    blocking("cleanup_report", move || {
        let rules = rules_of(&core);
        let conn = core.db()?;
        cleanup::report(&conn, &core.roots, &rules)
    })
    .await
}

#[tauri::command]
pub async fn list_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    let core = core_of(&state);
    blocking("list_templates", move || {
        let conn = core.db()?;
        templates::list(&conn)
    })
    .await
}

#[tauri::command]
pub async fn add_template(state: State<'_, AppState>, path: String) -> Result<Template> {
    let core = core_of(&state);
    blocking("add_template", move || {
        let conn = core.db()?;
        templates::add(&conn, &core.roots, Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn remove_template(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core_of(&state);
    blocking("remove_template", move || {
        let conn = core.db()?;
        templates::remove(&conn, id)
    })
    .await
}

/// A fresh copy of a template: where it is, and whether Windows opened it,
/// so the renderer can word its toast (audit T-19).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateCopy {
    pub path: String,
    pub opened: bool,
}

#[tauri::command]
pub async fn use_template(state: State<'_, AppState>, app: tauri::AppHandle, id: i64) -> Result<TemplateCopy> {
    use tauri_plugin_opener::OpenerExt;
    let core = core_of(&state);
    blocking("use_template", move || {
        let copy = {
            let conn = core.db()?;
            templates::use_template(&conn, &core.roots, id)?
        };
        let path = paths::display(&copy);
        let opened = match app.opener().open_path(&path, None::<&str>) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("use_template: the copy could not be opened ({error})");
                false
            }
        };
        Ok(TemplateCopy { path, opened })
    })
    .await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    let core = core_of(&state);
    blocking("get_settings", move || Ok(settings_of(&core))).await
}

#[tauri::command]
pub async fn set_settings(state: State<'_, AppState>, app: tauri::AppHandle, settings: Settings) -> Result<Settings> {
    let core = core_of(&state);
    blocking("set_settings", move || {
        // Folder choices only change through add_folder and remove_folder.
        let current = settings_of(&core);
        let settings = Settings { extra_folders: current.extra_folders, excluded_folders: current.excluded_folders, ..settings };
        save_settings(&core, &settings)?;
        apply_autostart(&app, settings.start_at_login);
        Ok(settings)
    })
    .await
}

/// Adds a folder Tote should look in. Takes effect after a restart, because
/// the watchers and the scan are built once at startup.
#[tauri::command]
pub async fn add_folder(state: State<'_, AppState>, path: String) -> Result<Settings> {
    let core = core_of(&state);
    blocking("add_folder", move || {
        let folder = paths::resolve(Path::new(&path))?;
        if !folder.is_dir() {
            return Err(ToteError::new(ErrorCode::NotFound, "That folder could not be found. Pick another folder.").with_path(path));
        }
        let mut settings = settings_of(&core);
        check_folder_choice(&folder, &covered_folders(core.roots.all(), &settings), dirs::home_dir().as_deref())?;
        choose_folder(&mut settings, &folder);
        save_settings(&core, &settings)?;
        Ok(settings)
    })
    .await
}

/// The folders a choice is checked against: the roots Tote started with
/// plus the folders added since, which only become roots at the next start.
fn covered_folders(roots: Vec<PathBuf>, settings: &Settings) -> Vec<PathBuf> {
    let mut covered = roots;
    covered.extend(settings.extra_folders.iter().map(PathBuf::from));
    covered
}

/// A folder she may add: not a whole drive, not her home folder, not above
/// a folder Tote already looks in (its file commands would reach all of it,
/// and the scan would cover it twice), and not one already covered.
fn check_folder_choice(folder: &Path, roots: &[PathBuf], home: Option<&Path>) -> Result<()> {
    let whole_drive = folder.parent().is_none();
    let above_a_root = roots.iter().any(|r| !paths::same_path(r, folder) && r.starts_with(folder));
    if whole_drive || home.is_some_and(|h| paths::same_path(h, folder)) || above_a_root {
        return Err(ToteError::validation("Pick a folder inside your files, not a whole drive."));
    }
    if roots.iter().any(|r| paths::same_path(r, folder) || folder.starts_with(r)) {
        return Err(ToteError::validation("Tote already looks in that folder. Pick another one."));
    }
    Ok(())
}

/// Records the choice: a default folder she switched off is switched back
/// on, and only that; any other folder joins the added list once.
fn choose_folder(settings: &mut Settings, folder: &Path) {
    let was_excluded = settings.excluded_folders.iter().any(|e| paths::same_path(Path::new(e), folder));
    settings.excluded_folders.retain(|e| !paths::same_path(Path::new(e), folder));
    if !was_excluded && !settings.extra_folders.iter().any(|e| paths::same_path(Path::new(e), folder)) {
        settings.extra_folders.push(paths::display(folder));
    }
}

/// Switches a folder off. Documents and Downloads cannot be removed.
#[tauri::command]
pub async fn remove_folder(state: State<'_, AppState>, path: String) -> Result<Settings> {
    let core = core_of(&state);
    blocking("remove_folder", move || {
        let folder = Path::new(&path);
        if !core.roots.removable(folder) {
            return Err(ToteError::validation("Documents and Downloads are where filing happens, so they stay. Remove a different folder instead."));
        }
        let mut settings = settings_of(&core);
        let was_extra = settings.extra_folders.iter().any(|e| paths::same_path(Path::new(e), folder));
        settings.extra_folders.retain(|e| !paths::same_path(Path::new(e), folder));
        if !was_extra && !settings.excluded_folders.iter().any(|e| paths::same_path(Path::new(e), folder)) {
            settings.excluded_folders.push(path.clone());
        }
        save_settings(&core, &settings)?;
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

pub fn apply_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    if paths::Roots::test_mode() {
        log::info!("test mode (TOTE_ROOTS): autostart left untouched");
        return;
    }
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    if let Err(error) = result {
        log::warn!("autostart: {error}");
    }
}

/// The newer version to offer, if any (update.rs). Goes online at most once
/// a day and never while she has switched the check off. The database is
/// only touched for two short settings reads and writes, never across an
/// await, so this stays on the async runtime rather than the blocking pool.
#[tauri::command]
pub async fn check_update(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<Option<crate::update::UpdateInfo>> {
    let core = state.0.clone();
    let enabled = settings_of(&core).check_updates;
    log_failure("check_update", crate::update::check(&app, &core, enabled).await)
}

/// Downloads, verifies and installs the newest version. On Windows the
/// installer closes Tote and opens the new version.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<()> {
    log_failure("install_update", crate::update::install(&app).await)
}

/// Which browsers would save downloads outside Downloads (spec 5.1.2).
/// Always compared with the real Downloads folder, also in test mode,
/// because the browsers' settings refer to the real one.
#[tauri::command]
pub async fn browser_check() -> Result<Vec<crate::browsers::BrowserStatus>> {
    blocking("browser_check", move || {
        let downloads = dirs::download_dir().ok_or_else(|| ToteError::new(ErrorCode::NotFound, "The Downloads folder could not be found. Check it in File Explorer, then try again."))?;
        Ok(crate::browsers::check(&downloads, dirs::desktop_dir().as_deref()))
    })
    .await
}

/// What Organize would do right now, for her to check first (spec 5.11).
#[tauri::command]
pub async fn organize_plan(state: State<'_, AppState>) -> Result<crate::organize::Plan> {
    let core = core_of(&state);
    blocking("organize_plan", move || {
        let rules = rules_of(&core);
        crate::organize::plan(&core.roots, &rules)
    })
    .await
}

/// Moves the files she confirmed. Destinations are recomputed here.
#[tauri::command]
pub async fn organize_run(state: State<'_, AppState>, paths: Vec<String>) -> Result<crate::organize::RunResult> {
    let core = core_of(&state);
    blocking("organize_run", move || {
        let rules = rules_of(&core);
        crate::organize::run(&core, &rules, &paths)
    })
    .await
}

/// One Undo for a whole Organize run.
#[tauri::command]
pub async fn undo_batch(state: State<'_, AppState>, batch: String) -> Result<ops::BatchUndo> {
    let core = core_of(&state);
    blocking("undo_batch", move || {
        let conn = core.db()?;
        ops::undo_batch(&conn, &core.roots, &core.placed, &batch)
    })
    .await
}

/// The standard subject list, for "Use the standard list" in Settings.
#[tauri::command]
pub fn default_subject_rules() -> Vec<crate::organize::SubjectRule> {
    crate::organize::default_rules()
}

#[tauri::command]
pub fn index_status(state: State<'_, AppState>) -> Result<IndexStatus> {
    Ok(state.0.status())
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<Task>> {
    let core = core_of(&state);
    blocking("list_tasks", move || {
        let conn = core.db()?;
        tasks::list(&conn, db::now())
    })
    .await
}

#[tauri::command]
pub async fn add_task(state: State<'_, AppState>, title: String, due: Option<String>) -> Result<Task> {
    let core = core_of(&state);
    blocking("add_task", move || {
        let conn = core.db()?;
        tasks::add(&conn, &title, due.as_deref(), db::now())
    })
    .await
}

#[tauri::command]
pub async fn update_task(state: State<'_, AppState>, id: i64, title: String, due: Option<String>, repeat: Repeat) -> Result<Task> {
    let core = core_of(&state);
    blocking("update_task", move || {
        // Settings first; settings_of takes its own lock.
        let quarters = settings_of(&core).quarters;
        let conn = core.db()?;
        tasks::update(&conn, id, &title, due.as_deref(), repeat, &quarters, db::now())
    })
    .await
}

#[tauri::command]
pub async fn complete_task(state: State<'_, AppState>, id: i64) -> Result<Completed> {
    let core = core_of(&state);
    blocking("complete_task", move || {
        let quarters = settings_of(&core).quarters;
        let conn = core.db()?;
        tasks::complete(&conn, id, &quarters, db::now())
    })
    .await
}

#[tauri::command]
pub async fn reopen_task(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core_of(&state);
    blocking("reopen_task", move || {
        let conn = core.db()?;
        tasks::reopen(&conn, id)
    })
    .await
}

#[tauri::command]
pub async fn delete_task(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core_of(&state);
    blocking("delete_task", move || {
        let conn = core.db()?;
        tasks::delete(&conn, id, db::now())
    })
    .await
}

#[tauri::command]
pub async fn restore_task(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core_of(&state);
    blocking("restore_task", move || {
        let conn = core.db()?;
        tasks::restore(&conn, id)
    })
    .await
}

#[tauri::command]
pub async fn attach_file(state: State<'_, AppState>, id: i64, path: String) -> Result<Task> {
    let core = core_of(&state);
    blocking("attach_file", move || {
        let conn = core.db()?;
        tasks::attach_file(&conn, &core.roots, id, Path::new(&path), db::now())
    })
    .await
}

#[tauri::command]
pub async fn detach_file(state: State<'_, AppState>, id: i64, path: String) -> Result<Task> {
    let core = core_of(&state);
    blocking("detach_file", move || {
        let conn = core.db()?;
        tasks::detach_file(&conn, id, &path)
    })
    .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_choice_is_checked_against_the_drive_the_home_and_the_roots() {
        let dir = tempfile::tempdir().unwrap();
        let base = paths::resolve(dir.path()).unwrap();
        let documents = base.join("Documents");
        let lessons = documents.join("Lessons");
        let other = base.join("Other");
        for d in [&lessons, &other, &base.join("Desktop"), &base.join("Downloads")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let roots = vec![base.join("Desktop"), documents.clone(), base.join("Downloads")];
        let home = base.join("Home");
        let drive = base.ancestors().last().unwrap().to_path_buf();
        let message = |folder: &Path| check_folder_choice(folder, &roots, Some(&home)).unwrap_err().message;
        let too_wide = "Pick a folder inside your files, not a whole drive.";
        assert_eq!(message(&drive), too_wide, "a drive root");
        assert_eq!(message(&home), too_wide, "the home folder");
        assert_eq!(message(&base), too_wide, "a parent of every root");
        let already = "Tote already looks in that folder. Pick another one.";
        assert_eq!(message(&documents), already);
        assert_eq!(message(&lessons), already);
        assert!(check_folder_choice(&other, &roots, Some(&home)).is_ok(), "a folder beside the roots");
        assert!(check_folder_choice(&other, &roots, None).is_ok());
        // A folder added earlier in the same session is a root only after
        // the restart; until then the saved choice counts the same way.
        let inside_other = other.join("Grade 7");
        std::fs::create_dir_all(&inside_other).unwrap();
        let pending = |folder: &Path| Settings { extra_folders: vec![paths::display(folder)], ..Settings::default() };
        let with_other = covered_folders(roots.clone(), &pending(&other));
        assert_eq!(check_folder_choice(&inside_other, &with_other, Some(&home)).unwrap_err().message, already, "inside a folder added this session");
        assert_eq!(check_folder_choice(&other, &with_other, Some(&home)).unwrap_err().message, already, "the folder added this session itself");
        let with_inside = covered_folders(roots.clone(), &pending(&inside_other));
        assert_eq!(check_folder_choice(&other, &with_inside, Some(&home)).unwrap_err().message, too_wide, "above a folder added this session");
        assert!(check_folder_choice(&base.join("Elsewhere"), &with_inside, Some(&home)).is_ok());
    }

    /// Settings text a newer or older Tote cannot read is kept aside once,
    /// the live row stays, and the defaults are used (audit T-47).
    #[test]
    fn unreadable_settings_are_kept_aside_once_before_the_defaults_are_used() {
        let conn = db::open_memory().unwrap();
        let bad = r#"{"startAtLogin":"yes","quarters":7}"#;
        db::set_setting(&conn, "settings", bad).unwrap();
        let settings = settings_from(&conn);
        assert_eq!(settings.start_at_login, Settings::default().start_at_login);
        assert!(settings.quarters.is_empty());
        settings_from(&conn);
        let kept: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT key, value FROM settings WHERE key LIKE 'settings.unreadable-%'").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(kept.len(), 1, "kept once, not on every read: {kept:?}");
        assert_eq!(kept[0].1, bad);
        assert_eq!(db::get_setting(&conn, "settings").unwrap().as_deref(), Some(bad), "the live row is left for a newer Tote to read");
        let good = serde_json::to_string(&Settings { tour_done: true, ..Settings::default() }).unwrap();
        db::set_setting(&conn, "settings", &good).unwrap();
        assert!(settings_from(&conn).tour_done, "readable settings are read as before");
        assert!(settings_from(&db::open_memory().unwrap()).start_at_login, "nothing stored means the defaults");
    }

    /// One field in the wrong shape resets only itself: the other fields
    /// survive, a missing field is its default, and the text is kept aside
    /// once because something was reset (audit T-47).
    #[test]
    fn an_unreadable_field_resets_only_itself() {
        let conn = db::open_memory().unwrap();
        let row = r#"{"startAtLogin":false,"quarters":"none","extraFolders":["D:\\Lessons"],"tourDone":true}"#;
        db::set_setting(&conn, "settings", row).unwrap();
        let settings = settings_from(&conn);
        assert!(!settings.start_at_login, "a good field survives");
        assert_eq!(settings.extra_folders, vec!["D:\\Lessons".to_string()]);
        assert!(settings.tour_done);
        assert!(settings.quarters.is_empty(), "the field that could not be read is its default");
        assert!(settings.notifications, "a missing field is its default");
        let kept = || -> Vec<String> {
            let mut stmt = conn.prepare("SELECT value FROM settings WHERE key LIKE 'settings.unreadable-%'").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(kept(), vec![row.to_string()], "the text is kept aside because a field was reset");
        // A row that only lacks a newer field reads clean and keeps nothing aside.
        db::set_setting(&conn, "settings", r#"{"tourDone":true}"#).unwrap();
        assert!(settings_from(&conn).tour_done);
        assert_eq!(kept().len(), 1);
        let (settings, reset) = read_settings("not json");
        assert!(settings.start_at_login);
        assert_eq!(reset, ["settings"]);
        assert_eq!(read_settings("[1, 2]").1, ["settings"]);
        assert_eq!(read_settings(r#"{"startAtLogin":"yes","quarters":7}"#).1, ["startAtLogin", "quarters"]);
    }

    /// The renderer words the template toast from `opened`.
    #[test]
    fn a_template_copy_says_whether_it_opened() {
        let json = serde_json::to_value(TemplateCopy { path: "C:\\Her\\Documents\\School Files\\Word\\2026-10\\Plan 2026-10-16.docx".into(), opened: false }).unwrap();
        assert_eq!(json["opened"], serde_json::json!(false));
        assert!(json["path"].as_str().unwrap().ends_with("Plan 2026-10-16.docx"));
    }

    #[test]
    fn re_adding_a_switched_off_default_only_clears_the_switch() {
        let mut settings = Settings { excluded_folders: vec!["C:\\Users\\Her\\Desktop".into()], ..Settings::default() };
        choose_folder(&mut settings, Path::new("c:\\users\\her\\desktop"));
        assert!(settings.excluded_folders.is_empty(), "the switch is cleared");
        assert!(settings.extra_folders.is_empty(), "a default folder is never listed as an added one");
        choose_folder(&mut settings, Path::new("D:\\Lessons"));
        choose_folder(&mut settings, Path::new("d:\\lessons"));
        assert_eq!(settings.extra_folders, vec!["D:\\Lessons".to_string()], "an added folder is listed once");
    }
}
