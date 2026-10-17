//! Whether a file is ready to move: no browser is still writing it, and no
//! other program holds it open.

use std::path::Path;

/// True while a browser is still writing the download beside this file:
/// Firefox keeps an empty `X.pdf` next to the growing `X.pdf.part` until the
/// end, and Chrome names its part file `X.pdf.crdownload` (audit T-15).
pub fn has_partial_sibling(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { return false };
    ["part", "crdownload"].iter().any(|ext| path.with_file_name(format!("{name}.{ext}")).exists())
}

/// True when the file can be opened with no sharing, meaning no other program
/// has it open. On other platforms a plain open stands in.
pub fn can_open_exclusively(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(path).is_ok()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(path).is_ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_file_opens_exclusively() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("done.docx");
        std::fs::write(&file, b"x").unwrap();
        assert!(can_open_exclusively(&file));
    }

    #[test]
    fn a_file_with_a_part_or_crdownload_sibling_is_still_downloading() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("DepEd Memo 123.pdf");
        std::fs::write(&file, b"").unwrap();
        assert!(!has_partial_sibling(&file));
        std::fs::write(dir.path().join("DepEd Memo 123.pdf.part"), b"half").unwrap();
        assert!(has_partial_sibling(&file));
        std::fs::remove_file(dir.path().join("DepEd Memo 123.pdf.part")).unwrap();
        assert!(!has_partial_sibling(&file));
        std::fs::write(dir.path().join("DepEd Memo 123.pdf.crdownload"), b"half").unwrap();
        assert!(has_partial_sibling(&file));
        assert!(!has_partial_sibling(&dir.path().join("Other.pdf")));
    }
}
