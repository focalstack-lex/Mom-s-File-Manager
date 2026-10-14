//! Files on the clipboard as files (CF_HDROP), so she can paste a file into a
//! Messenger or Gmail message straight from a result row (spec 5.3).

use crate::error::{ToteError, ErrorCode, Result};
use std::path::Path;

#[cfg(windows)]
pub fn copy_files(paths: &[std::path::PathBuf]) -> Result<()> {
    use clipboard_win::{formats, Clipboard, Setter};
    let list: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let busy = |e: clipboard_win::ErrorCode| ToteError::new(ErrorCode::Io, format!("Windows did not let Tote use the clipboard: {e}"));
    let _open = Clipboard::new_attempts(10).map_err(busy)?;
    formats::FileList.write_clipboard(list.as_slice()).map_err(busy)
}

#[cfg(not(windows))]
pub fn copy_files(_paths: &[std::path::PathBuf]) -> Result<()> {
    Err(ToteError::new(ErrorCode::Unsupported, "Copying files to the clipboard is only available on Windows."))
}

/// Every path must exist before it is offered to another program.
pub fn check_exists(paths: &[std::path::PathBuf]) -> Result<()> {
    for path in paths {
        if !Path::new(path).exists() {
            return Err(ToteError::new(ErrorCode::NotFound, "That file is no longer there.").with_path(path.to_string_lossy()));
        }
    }
    Ok(())
}
