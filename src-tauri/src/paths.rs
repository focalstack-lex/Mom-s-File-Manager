//! Known folders, the indexed roots, and the one boundary check every file
//! command runs: a path is only acted on when it resolves inside a root.

use crate::error::{DeskError, ErrorCode, Result};
use std::path::{Component, Path, PathBuf};

/// The folders the app indexes and may change files in (spec 5.2).
#[derive(Debug, Clone)]
pub struct Roots {
    pub desktop: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    pub onedrive: Option<PathBuf>,
}

impl Roots {
    /// Resolves the roots through the platform's known-folder API.
    pub fn detect() -> Self {
        let onedrive = dirs::home_dir().map(|home| home.join("OneDrive")).filter(|p| p.is_dir());
        Self {
            desktop: dirs::desktop_dir(),
            documents: dirs::document_dir(),
            downloads: dirs::download_dir(),
            onedrive,
        }
    }

    /// Roots built from explicit folders, used by tests.
    #[cfg(test)]
    pub fn explicit(desktop: PathBuf, documents: PathBuf, downloads: PathBuf) -> Self {
        Self { desktop: Some(desktop), documents: Some(documents), downloads: Some(downloads), onedrive: None }
    }

    /// Every root that exists, in display order.
    pub fn all(&self) -> Vec<PathBuf> {
        [&self.desktop, &self.documents, &self.downloads, &self.onedrive]
            .into_iter()
            .flatten()
            .filter(|p| p.is_dir())
            .cloned()
            .collect()
    }

    /// `Documents\School Files`, where sorted downloads go (spec 5.1).
    pub fn school_files(&self) -> Result<PathBuf> {
        self.documents
            .as_ref()
            .map(|d| d.join("School Files"))
            .ok_or_else(|| DeskError::new(ErrorCode::NotFound, "The Documents folder could not be found."))
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
        Err(DeskError::new(
            ErrorCode::OutsideRoots,
            "Desk only works with files in Desktop, Documents, Downloads and OneDrive.",
        )
        .with_path(display(path)))
    }
}

/// Canonicalizes the deepest existing ancestor and appends the rest, with the
/// Windows `\\?\` prefix removed so paths stay readable.
pub fn resolve(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(DeskError::new(ErrorCode::InvalidPath, "The path must be absolute.").with_path(display(path)));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(DeskError::new(ErrorCode::InvalidPath, "The path must not contain '..'.").with_path(display(path)));
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
                return Err(DeskError::new(ErrorCode::NotFound, "The folder does not exist.").with_path(display(path)))
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
        return Err(DeskError::new(ErrorCode::InvalidName, "Type a name first."));
    }
    if trimmed.len() > 200 {
        return Err(DeskError::new(ErrorCode::InvalidName, "That name is too long. Keep it under 200 characters."));
    }
    if let Some(bad) = trimmed.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        return Err(DeskError::new(
            ErrorCode::InvalidName,
            format!("Names cannot contain {bad}. Try a space or a hyphen instead."),
        ));
    }
    if trimmed.ends_with('.') || trimmed != name {
        return Err(DeskError::new(ErrorCode::InvalidName, "Names cannot start or end with a space or a period."));
    }
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(DeskError::new(ErrorCode::InvalidName, format!("{trimmed} is a name Windows reserves. Pick another.")));
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
