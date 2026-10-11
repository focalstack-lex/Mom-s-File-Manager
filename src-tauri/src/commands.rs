//! The command layer: validates input, calls the modules, and returns plain
//! data. The renderer never touches the filesystem except through here.

use crate::cleanup::{self, CleanupReport};
use crate::clipboard;
use crate::db;
use crate::error::{DeskError, ErrorCode, Result};
use crate::extract;
use crate::index::{self, FileRow, Filters, Group, IndexStatus};
use crate::ops::{self, OpRow};
use crate::paths;
use crate::sorter;
use crate::state::{AppState, Core};
use crate::templates::{self, Template};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
}

impl Default for Settings {
    fn default() -> Self {
        Self { start_at_login: true, notifications: true, tour_done: false, school_year: None, quarters: Vec::new() }
    }
}

pub fn settings_of(core: &Core) -> Settings {
    let Ok(conn) = core.db() else { return Settings::default() };
    match db::get_setting(&conn, "settings") {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Settings::default(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub roots: Vec<String>,
    pub school_files: String,
    pub downloads: Option<String>,
    pub settings: Settings,
    pub index: IndexStatus,
    pub sorted_this_week: usize,
    pub recovered: bool,
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

fn core<'a>(state: &'a State<'_, AppState>) -> &'a Core {
    &state.0
}

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    let core = core(&state);
    let conn = core.db()?;
    let week_ago = db::now() - 7 * 86_400;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        roots: core.roots.all().iter().map(|p: &PathBuf| paths::display(p)).collect(),
        school_files: core.roots.school_files().map(|p: PathBuf| paths::display(&p)).unwrap_or_default(),
        downloads: core.roots.downloads.as_deref().map(|p: &Path| paths::display(p)),
        settings: settings_of(core),
        index: core.status(),
        sorted_this_week: index::sorted_since(&conn, week_ago)?,
        recovered: core.recovered,
    })
}

#[tauri::command]
pub fn find(state: State<'_, AppState>, query: String, filters: Filters) -> Result<FindResult> {
    let core = core(&state);
    let conn = core.db()?;
    let has_filters = filters.from.is_some() || filters.to.is_some() || !filters.kinds.is_empty() || !filters.sources.is_empty();
    let searching = !query.trim().is_empty() || has_filters;
    let groups = if searching { index::group(index::search(&conn, &query, &filters, 200)?) } else { Vec::new() };
    Ok(FindResult {
        groups,
        pinned: if searching { Vec::new() } else { index::pinned(&conn)? },
        orphan_pins: if searching { Vec::new() } else { index::orphan_pins(&conn)? },
        recent: if searching { Vec::new() } else { index::recent_downloads(&conn, 30, 100)? },
        index: core.status(),
    })
}

#[tauri::command]
pub fn set_pinned(state: State<'_, AppState>, path: String, pinned: bool) -> Result<()> {
    let core = core(&state);
    let confined = core.roots.confine(Path::new(&path))?;
    let conn = core.db()?;
    index::set_pinned(&conn, &paths::display(&confined), pinned)
}

#[tauri::command]
pub fn forget_pin(state: State<'_, AppState>, path: String) -> Result<()> {
    let conn = core(&state).db()?;
    index::set_pinned(&conn, &path, false)
}

#[tauri::command]
pub fn open_path(state: State<'_, AppState>, app: tauri::AppHandle, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let confined = core(&state).roots.confine(Path::new(&path))?;
    if !confined.exists() {
        return Err(DeskError::new(ErrorCode::NotFound, "That file is no longer there.").with_path(path));
    }
    app.opener()
        .open_path(paths::display(&confined), None::<&str>)
        .map_err(|e| DeskError::new(ErrorCode::Io, format!("Windows could not open it: {e}")))
}

#[tauri::command]
pub fn reveal_path(state: State<'_, AppState>, app: tauri::AppHandle, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let confined = core(&state).roots.confine(Path::new(&path))?;
    if !confined.exists() {
        return Err(DeskError::new(ErrorCode::NotFound, "That file is no longer there.").with_path(path));
    }
    app.opener()
        .reveal_item_in_dir(&confined)
        .map_err(|e| DeskError::new(ErrorCode::Io, format!("Windows could not show it: {e}")))
}

#[tauri::command]
pub fn open_school_files(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let school = core(&state).roots.school_files()?;
    std::fs::create_dir_all(&school)?;
    app.opener()
        .open_path(paths::display(&school), None::<&str>)
        .map_err(|e| DeskError::new(ErrorCode::Io, format!("Windows could not open the folder: {e}")))
}

#[tauri::command]
pub fn copy_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<()> {
    let core = core(&state);
    let confined: Vec<PathBuf> = paths.iter().map(|p| core.roots.confine(Path::new(p))).collect::<Result<_>>()?;
    clipboard::check_exists(&confined)?;
    clipboard::copy_files(&confined)
}

#[tauri::command]
pub fn preview(state: State<'_, AppState>, path: String) -> Result<Preview> {
    let core = core(&state);
    let confined = core.roots.confine(Path::new(&path))?;
    let meta = std::fs::metadata(&confined)?;
    let ext = sorter::extension(&confined);
    let kind = sorter::classify(&ext);
    let name = confined.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = confined.parent().map(paths::display).unwrap_or_default();
    let modified = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (kind_text, text) = if kind == sorter::Kind::Images {
        ("image".to_string(), None)
    } else {
        match extract::extract_text(&confined, &ext, meta.len()) {
            Some(t) => ("text".to_string(), Some(t.chars().take(4000).collect())),
            None => ("details".to_string(), None),
        }
    };
    Ok(Preview { kind: kind_text, text, name, size: meta.len(), modified, parent })
}

#[tauri::command]
pub fn move_files(state: State<'_, AppState>, paths: Vec<String>, destination: String) -> Result<Vec<String>> {
    let core = core(&state);
    let conn = core.db()?;
    let dest = Path::new(&destination);
    let mut moved = Vec::new();
    for p in paths {
        let new_path = ops::move_file(&conn, &core.roots, Path::new(&p), dest, ops::OpKind::Move)?;
        moved.push(paths::display(&new_path));
    }
    Ok(moved)
}

#[tauri::command]
pub fn rename_file(state: State<'_, AppState>, path: String, new_stem: String) -> Result<String> {
    let core = core(&state);
    let conn = core.db()?;
    Ok(paths::display(&ops::rename_file(&conn, &core.roots, Path::new(&path), &new_stem)?))
}

#[tauri::command]
pub fn create_folder(state: State<'_, AppState>, parent: String, name: String) -> Result<String> {
    let core = core(&state);
    let conn = core.db()?;
    Ok(paths::display(&ops::create_folder(&conn, &core.roots, Path::new(&parent), &name)?))
}

#[tauri::command]
pub fn recycle_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<OpRow>> {
    let core = core(&state);
    let conn = core.db()?;
    let targets: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    ops::recycle(&conn, &core.roots, &targets)
}

/// Folders she can move things into: School Files subfolders first, recent
/// destinations marked, then the top-level folders inside each root.
#[tauri::command]
pub fn list_destinations(state: State<'_, AppState>) -> Result<Vec<Destination>> {
    let core = core(&state);
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
}

#[tauri::command]
pub fn list_activity(state: State<'_, AppState>) -> Result<Vec<OpRow>> {
    let conn = core(&state).db()?;
    ops::list(&conn, 200)
}

#[tauri::command]
pub fn undo(state: State<'_, AppState>, id: i64) -> Result<()> {
    let core = core(&state);
    let conn = core.db()?;
    ops::undo(&conn, &core.roots, id)
}

#[tauri::command]
pub fn cleanup_report(state: State<'_, AppState>) -> Result<CleanupReport> {
    let core = core(&state);
    let conn = core.db()?;
    cleanup::report(&conn, &core.roots)
}

#[tauri::command]
pub fn list_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    let conn = core(&state).db()?;
    templates::list(&conn)
}

#[tauri::command]
pub fn add_template(state: State<'_, AppState>, path: String) -> Result<Template> {
    let core = core(&state);
    let conn = core.db()?;
    templates::add(&conn, &core.roots, Path::new(&path))
}

#[tauri::command]
pub fn remove_template(state: State<'_, AppState>, id: i64) -> Result<()> {
    let conn = core(&state).db()?;
    templates::remove(&conn, id)
}

#[tauri::command]
pub fn use_template(state: State<'_, AppState>, app: tauri::AppHandle, id: i64) -> Result<String> {
    use tauri_plugin_opener::OpenerExt;
    let core = core(&state);
    let conn = core.db()?;
    let copy = templates::use_template(&conn, &core.roots, id)?;
    let text = paths::display(&copy);
    if let Err(error) = app.opener().open_path(&text, None::<&str>) {
        log::warn!("open template copy: {error}");
    }
    Ok(text)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    Ok(settings_of(core(&state)))
}

#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, app: tauri::AppHandle, settings: Settings) -> Result<Settings> {
    let core = core(&state);
    let json = serde_json::to_string(&settings).map_err(|e| DeskError::validation(e.to_string()))?;
    let conn = core.db()?;
    db::set_setting(&conn, "settings", &json)?;
    apply_autostart(&app, settings.start_at_login);
    Ok(settings)
}

pub fn apply_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    if let Err(error) = result {
        log::warn!("autostart: {error}");
    }
}

#[tauri::command]
pub fn index_status(state: State<'_, AppState>) -> Result<IndexStatus> {
    Ok(core(&state).status())
}
