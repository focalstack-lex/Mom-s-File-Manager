//! Where a file belongs (spec 5.1): type folder, month folder, and a name that
//! never overwrites. Moving itself is in `ops`, so every move is logged.

use chrono::{DateTime, Local};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The type folders under School Files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Word,
    Pdf,
    Excel,
    PowerPoint,
    Images,
    Videos,
    Other,
}

impl Kind {
    pub fn folder_name(self) -> &'static str {
        match self {
            Kind::Word => "Word",
            Kind::Pdf => "PDF",
            Kind::Excel => "Excel",
            Kind::PowerPoint => "PowerPoint",
            Kind::Images => "Images",
            Kind::Videos => "Videos",
            Kind::Other => "Other",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Word => "word",
            Kind::Pdf => "pdf",
            Kind::Excel => "excel",
            Kind::PowerPoint => "powerpoint",
            Kind::Images => "images",
            Kind::Videos => "videos",
            Kind::Other => "other",
        }
    }

    /// The kind in words for a log line that must not carry the file's name
    /// (names often hold a student's name, audit T-52).
    pub fn words(self) -> &'static str {
        match self {
            Kind::Word => "a Word file",
            Kind::Pdf => "a PDF file",
            Kind::Excel => "an Excel file",
            Kind::PowerPoint => "a PowerPoint file",
            Kind::Images => "an image",
            Kind::Videos => "a video",
            Kind::Other => "a file",
        }
    }
}

/// "a Word file", "an image": how a log line names a file without its name.
pub fn describe(path: &Path) -> &'static str {
    classify(&extension(path)).words()
}

/// Lowercase extension without the dot, or empty.
pub fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The type folder for an extension. Every file has a kind; whether it is
/// sorted automatically is a separate question (`stays_in_downloads`).
pub fn classify(ext: &str) -> Kind {
    match ext {
        "doc" | "docx" | "rtf" | "odt" => Kind::Word,
        "pdf" => Kind::Pdf,
        "xls" | "xlsx" | "csv" | "ods" => Kind::Excel,
        "ppt" | "pptx" | "odp" => Kind::PowerPoint,
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "webp" | "bmp" => Kind::Images,
        "mp4" | "mov" | "avi" | "mkv" | "wmv" => Kind::Videos,
        _ => Kind::Other,
    }
}

/// Installers, scripts and archives are run or extracted once, so sorting them
/// only adds clutter. Anything Windows would run or open as a web page
/// (scripts, shortcuts, saved pages) stays too, so a file from the web is
/// never filed among her school work with a plain Open (audit T-41).
/// Partial downloads are not files yet.
pub fn stays_in_downloads(ext: &str) -> bool {
    matches!(
        ext,
        "exe"
            | "msi"
            | "bat"
            | "cmd"
            | "ps1"
            | "vbs"
            | "js"
            | "jse"
            | "wsf"
            | "hta"
            | "lnk"
            | "url"
            | "scr"
            | "com"
            | "reg"
            | "html"
            | "htm"
            | "zip"
            | "rar"
            | "7z"
            | "crdownload"
            | "part"
            | "tmp"
            | "download"
    )
}

/// `School Files\<Type>\<YYYY-MM>` for a file of `kind` at `when`.
pub fn destination(school_files: &Path, kind: Kind, when: DateTime<Local>) -> PathBuf {
    school_files.join(kind.folder_name()).join(when.format("%Y-%m").to_string())
}

/// The same path, or `Name (2).ext`, `Name (3).ext` and so on until the name is free.
pub fn unique_path(wanted: &Path) -> PathBuf {
    if !wanted.exists() {
        return wanted.to_path_buf();
    }
    let stem = wanted.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = wanted.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let parent = wanted.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} ({}){ext}", chrono::Utc::now().timestamp()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify("docx"), Kind::Word);
        assert_eq!(classify("PDF".to_ascii_lowercase().as_str()), Kind::Pdf);
        assert_eq!(classify("csv"), Kind::Excel);
        assert_eq!(classify("pptx"), Kind::PowerPoint);
        assert_eq!(classify("jpg"), Kind::Images);
        assert_eq!(classify("mp4"), Kind::Videos);
        assert_eq!(classify("xyz"), Kind::Other);
        assert!(stays_in_downloads("exe"));
        assert!(stays_in_downloads("crdownload"));
        assert!(!stays_in_downloads("docx"));
    }

    /// Scripts, shortcuts and saved web pages are runnable or open in a
    /// browser, so they stay in Downloads beside the installers (audit T-41).
    #[test]
    fn runnable_files_and_web_pages_stay_in_downloads() {
        for ext in ["vbs", "js", "jse", "wsf", "hta", "lnk", "url", "scr", "com", "cmd", "reg", "html", "htm", "bat", "ps1", "msi"] {
            assert!(stays_in_downloads(ext), "{ext} must stay in Downloads");
        }
        for ext in ["docx", "pdf", "xlsx", "pptx", "jpg", "mp4", "txt", "csv"] {
            assert!(!stays_in_downloads(ext), "{ext} is school work and is filed");
        }
    }

    #[test]
    fn a_log_line_names_the_kind_not_the_file() {
        assert_eq!(describe(Path::new(r"C:\Downloads\Santos, Juan grades.xlsx")), "an Excel file");
        assert_eq!(describe(Path::new(r"C:\Downloads\Quiz.docx")), "a Word file");
        assert_eq!(describe(Path::new(r"C:\Downloads\photo.JPG")), "an image");
        assert_eq!(describe(Path::new(r"C:\Downloads\notes.txt")), "a file");
    }

    #[test]
    fn destination_is_type_then_month() {
        let when = Local.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let dest = destination(Path::new(r"C:\Docs\School Files"), Kind::Word, when);
        assert_eq!(dest, PathBuf::from(r"C:\Docs\School Files\Word\2026-10"));
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("Quiz.docx");
        assert_eq!(unique_path(&wanted), wanted);
        std::fs::write(&wanted, b"x").unwrap();
        assert_eq!(unique_path(&wanted), dir.path().join("Quiz (2).docx"));
        std::fs::write(dir.path().join("Quiz (2).docx"), b"x").unwrap();
        assert_eq!(unique_path(&wanted), dir.path().join("Quiz (3).docx"));
    }
}
