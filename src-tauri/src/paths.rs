//! Known folders, the indexed roots, and the one boundary check every file
//! command runs: a path is only acted on when it resolves inside a root.

use crate::error::{ToteError, ErrorCode, Result};
use std::path::{Component, Path, PathBuf};

/// The folders the app indexes and may change files in (spec 5.2).
#[derive(Debug, Clone)]
pub struct Roots {
    pub desktop: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    pub onedrive: Option<PathBuf>,
    /// Pictures, Videos and Music: searched, and downloads saved there are
    /// noticed, but nothing in them is ever moved automatically.
    pub media: Vec<PathBuf>,
    /// Folders she added in Settings.
    pub extra: Vec<PathBuf>,
}

/// What Settings says about the roots: folders added, defaults switched off.
#[derive(Debug, Clone, Default)]
pub struct RootChoices {
    pub extra: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
}

impl Roots {
    /// Resolves the roots through the platform's known-folder API, honouring
    /// the Settings choices. `TOTE_ROOTS` (test mode) replaces all of it.
    pub fn detect(choices: &RootChoices) -> Self {
        if let Some(roots) = Self::from_env() {
            return roots;
        }
        let keep = |folder: Option<PathBuf>| folder.filter(|p| !choices.excluded.iter().any(|e| same_path(e, p)));
        let onedrive = dirs::home_dir().map(|home| home.join("OneDrive")).filter(|p| p.is_dir());
        let media = [dirs::picture_dir(), dirs::video_dir(), dirs::audio_dir()].into_iter().filter_map(keep).collect();
        Self {
            desktop: keep(dirs::desktop_dir()),
            documents: dirs::document_dir(),
            downloads: dirs::download_dir(),
            onedrive: keep(onedrive),
            media,
            extra: choices.extra.iter().filter(|p| p.is_dir()).cloned().collect(),
        }
    }

    /// `TOTE_ROOTS=<desktop>;<documents>;<downloads>[;<extra>...]` points the
    /// whole app at copied folders for a dry run (spec 8). Nothing real is touched.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("TOTE_ROOTS").ok()?;
        let parts: Vec<PathBuf> = raw.split(';').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        if parts.len() < 3 {
            log::error!("TOTE_ROOTS needs desktop;documents;downloads, got {raw:?}");
            return None;
        }
        Some(Self {
            desktop: Some(parts[0].clone()),
            documents: Some(parts[1].clone()),
            downloads: Some(parts[2].clone()),
            onedrive: None,
            media: Vec::new(),
            extra: parts[3..].to_vec(),
        })
    }

    /// True while `TOTE_ROOTS` is set: no autostart registration, separate data.
    pub fn test_mode() -> bool {
        std::env::var_os("TOTE_ROOTS").is_some()
    }

    /// Roots built from explicit folders, used by tests.
    #[cfg(test)]
    pub fn explicit(desktop: PathBuf, documents: PathBuf, downloads: PathBuf) -> Self {
        Self { desktop: Some(desktop), documents: Some(documents), downloads: Some(downloads), onedrive: None, media: Vec::new(), extra: Vec::new() }
    }

    /// Every root that exists, in display order, in resolved form so the
    /// paths the scanner stores match the ones `confine` produces for moves.
    pub fn all(&self) -> Vec<PathBuf> {
        [&self.desktop, &self.documents, &self.downloads, &self.onedrive]
            .into_iter()
            .flatten()
            .chain(self.media.iter())
            .chain(self.extra.iter())
            .filter(|p| p.is_dir())
            .map(|p| resolve(p).unwrap_or_else(|_| p.clone()))
            .collect()
    }

    /// Documents and Downloads are where sorting happens, so they stay.
    /// Desktop, OneDrive and added folders can be switched off.
    pub fn removable(&self, path: &Path) -> bool {
        let wanted = resolve(path).unwrap_or_else(|_| path.to_path_buf());
        let required = [&self.documents, &self.downloads];
        !required
            .into_iter()
            .flatten()
            .map(|r| resolve(r).unwrap_or_else(|_| r.clone()))
            .any(|r| same_path(&r, &wanted) || same_path(&r, path))
    }

    /// `Documents\School Files`, where sorted downloads go (spec 5.1).
    pub fn school_files(&self) -> Result<PathBuf> {
        self.documents
            .as_ref()
            .map(|d| resolve(d).unwrap_or_else(|_| d.clone()).join("School Files"))
            .ok_or_else(|| ToteError::new(ErrorCode::NotFound, "The Documents folder could not be found. Check it in File Explorer, then try again."))
    }

    /// Resolves `path` and refuses it unless it sits inside one of the roots.
    /// The deepest existing ancestor is canonicalized so a destination that
    /// does not exist yet is still checked against the real folder it lands in.
    pub fn confine(&self, path: &Path) -> Result<PathBuf> {
        let resolved = resolve(path)?;
        for root in self.all() {
            if let Ok(root) = resolve(&root) {
                if resolved.starts_with(&root) {
                    return Ok(resolved);
                }
            }
        }
        Err(ToteError::new(
            ErrorCode::OutsideRoots,
            "Tote only works with files in the folders listed in Settings. Add that folder there first.",
        )
        .with_path(display(path)))
    }
}

/// Windows paths compare without case.
pub fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().trim_end_matches('\\').eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches('\\'))
}

/// Canonicalizes the deepest existing ancestor and appends the rest, with the
/// Windows `\\?\` prefix removed so paths stay readable.
pub fn resolve(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(ToteError::new(ErrorCode::InvalidPath, "That file's location is not complete. Pick the file again in Find.").with_path(display(path)));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ToteError::new(ErrorCode::InvalidPath, "That file's location steps outside its folder. Pick the file again in Find.").with_path(display(path)));
    }
    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => {
                return Err(ToteError::new(ErrorCode::NotFound, "That folder could not be found. Check it in File Explorer, then try again.").with_path(display(path)))
            }
        }
    }
    let mut canonical = strip_verbatim(std::fs::canonicalize(&existing)?);
    for part in tail.iter().rev() {
        canonical.push(part);
    }
    Ok(canonical)
}

/// Removes the `\\?\` verbatim prefix `canonicalize` adds on Windows.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => PathBuf::from(format!(r"\\{rest}")),
        None => match text.strip_prefix(r"\\?\") {
            Some(rest) => PathBuf::from(rest),
            None => path,
        },
    }
}

pub fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Checks a file or folder name the user typed (spec 5.4).
pub fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ToteError::new(ErrorCode::InvalidName, "Type a name first."));
    }
    if trimmed.len() > 200 {
        return Err(ToteError::new(ErrorCode::InvalidName, "That name is too long. Keep it under 200 characters."));
    }
    if let Some(bad) = trimmed.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        return Err(ToteError::new(
            ErrorCode::InvalidName,
            format!("Names cannot contain {bad}. Try a space or a hyphen instead."),
        ));
    }
    if trimmed.ends_with('.') || trimmed != name {
        return Err(ToteError::new(ErrorCode::InvalidName, "Names cannot start or end with a space or a period. Remove it and try again."));
    }
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(ToteError::new(ErrorCode::InvalidName, format!("{trimmed} is a name Windows reserves. Pick another.")));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn confine_accepts_inside_and_rejects_outside() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = dir.path().join("Desktop");
        let documents = dir.path().join("Documents");
        let downloads = dir.path().join("Downloads");
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        let roots = Roots::explicit(desktop.clone(), documents.clone(), downloads.clone());
        let inside = documents.join("School Files").join("Word").join("new.docx");
        assert!(roots.confine(&inside).is_ok());
        let outside = dir.path().join("Elsewhere").join("x.txt");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        assert_eq!(roots.confine(&outside).unwrap_err().code, ErrorCode::OutsideRoots);
        let sneaky = documents.join("..").join("Elsewhere").join("x.txt");
        assert_eq!(roots.confine(&sneaky).unwrap_err().code, ErrorCode::InvalidPath);
    }

    #[test]
    fn extra_roots_count_and_required_roots_stay() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = dir.path().join("Desktop");
        let documents = dir.path().join("Documents");
        let downloads = dir.path().join("Downloads");
        let extra = dir.path().join("Lesson Plans");
        for d in [&desktop, &documents, &downloads, &extra] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut roots = Roots::explicit(desktop.clone(), documents.clone(), downloads.clone());
        roots.extra.push(extra.clone());
        assert_eq!(roots.all().len(), 4);
        assert!(roots.confine(&extra.join("plan.docx")).is_ok());
        assert!(roots.removable(&desktop));
        assert!(roots.removable(&extra));
        assert!(!roots.removable(&documents));
        assert!(!roots.removable(&PathBuf::from(downloads.to_string_lossy().to_uppercase())));
    }

    #[test]
    fn names_follow_windows_rules() {
        assert!(validate_name("Grade 7 Quiz").is_ok());
        assert_eq!(validate_name("").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(validate_name("a:b").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(validate_name("CON").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(validate_name("con.docx").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(validate_name("trailing.").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(validate_name(" lead").unwrap_err().code, ErrorCode::InvalidName);
    }

    #[test]
    fn strip_verbatim_removes_the_prefix() {
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(strip_verbatim(PathBuf::from(r"C:\x")), PathBuf::from(r"C:\x"));
    }
}
