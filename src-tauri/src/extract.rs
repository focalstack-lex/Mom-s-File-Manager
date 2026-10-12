//! Text inside documents (spec 5.2), so a search for a student's name finds
//! `Book1.xlsx`. Office files are zip archives of XML; the text is what is
//! left after the tags are removed. Every failure degrades to "name only".

use std::io::Read;
use std::path::Path;

/// Files larger than this are indexed by name only.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Extracted text is capped so one huge document cannot bloat the index.
pub const MAX_TEXT_CHARS: usize = 200_000;

/// Extracted text, or None when the type is unsupported or extraction failed.
///
/// Third-party readers panic on some malformed files (a scanned PDF on Lex's
/// machine made pdf-extract panic with "unexpected encoding", 2026-10-12).
/// Every extraction runs inside `catch_unwind`, which only works because the
/// release profile unwinds (Cargo.toml, guarded by a compile_error in lib.rs).
/// A file whose reader panics is indexed by name only.
pub fn extract_text(path: &Path, ext: &str, size: u64) -> Option<String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract_unguarded(path, ext, size))) {
        Ok(text) => text,
        Err(_) => {
            log::warn!("the reader for {} crashed on this file; it is searchable by name only", path.display());
            None
        }
    }
}

fn extract_unguarded(path: &Path, ext: &str, size: u64) -> Option<String> {
    if size > MAX_FILE_BYTES {
        return None;
    }
    let text = match ext {
        "docx" => ooxml_text(path, &["word/document.xml"]),
        "xlsx" => ooxml_text(path, &["xl/sharedStrings.xml"]),
        "pptx" => pptx_text(path),
        "pdf" => pdf_text(path),
        "txt" | "csv" | "md" => std::fs::read_to_string(path).ok(),
        _ => None,
    }?;
    Some(squeeze(&text))
}

fn ooxml_text(path: &Path, parts: &[&str]) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut out = String::new();
    for part in parts {
        if let Ok(mut entry) = archive.by_name(part) {
            let mut xml = String::new();
            if entry.read_to_string(&mut xml).is_ok() {
                out.push_str(&strip_tags(&xml));
                out.push(' ');
            }
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn pptx_text(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    let mut out = String::new();
    for name in names {
        if let Ok(mut entry) = archive.by_name(&name) {
            let mut xml = String::new();
            if entry.read_to_string(&mut xml).is_ok() {
                out.push_str(&strip_tags(&xml));
                out.push(' ');
            }
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn pdf_text(path: &Path) -> Option<String> {
    // Panics are caught one level up, in extract_text. The old version also
    // swapped the global panic hook per call, which raced with other threads.
    let bytes = std::fs::read(path).ok()?;
    match pdf_extract::extract_text_from_mem(&bytes) {
        Ok(text) if !text.trim().is_empty() => Some(text),
        _ => None,
    }
}

/// Removes XML tags, decoding the five basic entities, and keeps a space
/// between runs so words from adjacent cells do not fuse.
pub fn strip_tags(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() / 4);
    let mut in_tag = false;
    let mut entity = String::new();
    let mut in_entity = false;
    for c in xml.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                out.push(' ');
            }
            continue;
        }
        match c {
            '<' => in_tag = true,
            '&' => {
                in_entity = true;
                entity.clear();
            }
            ';' if in_entity => {
                in_entity = false;
                out.push_str(match entity.as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => " ",
                });
            }
            _ if in_entity => entity.push(c),
            _ => out.push(c),
        }
    }
    out
}

/// Collapses whitespace and caps the length.
fn squeeze(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT_CHARS));
    let mut last_space = true;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
        if out.len() >= MAX_TEXT_CHARS {
            break;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a minimal OOXML-style zip with the given parts.
    pub fn write_zip(path: &Path, parts: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in parts {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn docx_text_is_extracted_without_tags() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quiz.docx");
        write_zip(&path, &[("word/document.xml", "<w:p><w:r><w:t>Grade 7</w:t></w:r><w:r><w:t>Quiz &amp; Answers</w:t></w:r></w:p>")]);
        let size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(extract_text(&path, "docx", size).unwrap(), "Grade 7 Quiz & Answers");
    }

    #[test]
    fn xlsx_uses_shared_strings_and_pptx_reads_slides() {
        let dir = tempfile::tempdir().unwrap();
        let xlsx = dir.path().join("book1.xlsx");
        write_zip(&xlsx, &[("xl/sharedStrings.xml", "<sst><si><t>Santos</t></si><si><t>Reyes</t></si></sst>")]);
        assert_eq!(extract_text(&xlsx, "xlsx", 10).unwrap(), "Santos Reyes");
        let pptx = dir.path().join("deck.pptx");
        write_zip(&pptx, &[("ppt/slides/slide1.xml", "<a:t>Photosynthesis</a:t>"), ("ppt/slides/slide2.xml", "<a:t>Review</a:t>")]);
        assert_eq!(extract_text(&pptx, "pptx", 10).unwrap(), "Photosynthesis Review");
    }

    #[test]
    fn a_reader_that_panics_degrades_to_name_only() {
        // A PDF whose structure is half there: pdf-extract may error or panic;
        // either way extract_text must return None and the caller must live on.
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("scan.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n1 0 obj << /Type /Font /Subtype /Type1 /Encoding /StandardEncoding >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF").unwrap();
        assert_eq!(extract_text(&pdf, "pdf", 120), None);
        let caught = std::panic::catch_unwind(|| ());
        assert!(caught.is_ok(), "unwinding is available in this profile");
    }

    #[test]
    fn unsupported_corrupt_and_oversized_files_yield_none() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.docx");
        std::fs::write(&bad, b"not a zip").unwrap();
        assert_eq!(extract_text(&bad, "docx", 9), None);
        assert_eq!(extract_text(&bad, "doc", 9), None);
        assert_eq!(extract_text(&bad, "docx", MAX_FILE_BYTES + 1), None);
        let txt = dir.path().join("note.txt");
        std::fs::write(&txt, "hello\n\n  world").unwrap();
        assert_eq!(extract_text(&txt, "txt", 14).unwrap(), "hello world");
    }
}
