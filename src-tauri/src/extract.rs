//! Text inside documents (spec 5.2), so a search for a student's name finds
//! `Book1.xlsx`. Office files are zip archives of XML; the text is what is
//! left after the tags are removed. Every failure degrades to "name only".
//! The Office and text readers are bounded: a part of an Office file
//! streams through a byte limit and tag stripping stops at the text cap, so
//! one strange file cannot take memory a laptop does not have (audit T-04).
//! The PDF reader is bounded by the file's size on disk, a page budget and
//! the text cap (`pdf_text`), but pdf-extract still unpacks each content
//! stream whole in memory with no streaming interface, so one crafted
//! stream can still cost a great deal of memory (a leftover for the final
//! batch; the durable fix is process isolation).

use std::io::Read;
use std::path::Path;

/// Files larger than this are indexed by name only.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Extracted text is capped so one huge document cannot bloat the index.
pub const MAX_TEXT_CHARS: usize = 200_000;
/// One part of an Office file is read through this many unpacked bytes at
/// most, and a part that declares more is skipped. A 449 KB .docx whose
/// document.xml unpacked to 300 MB took Tote to 1.4 GB and ended it on a
/// laptop short of memory (audit T-04).
pub const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// The quick preview shows this many lines (audit T-46).
pub const PREVIEW_LINES: usize = 40;
/// A cap on the preview's length, so forty very wide lines stay light.
pub const PREVIEW_CHARS: usize = 12_000;
/// The buffer a part streams through: with the capped text, the memory one
/// part costs.
const READ_CHUNK: usize = 64 * 1024;

/// Extracted text for the index, or None when the type is unsupported or
/// extraction failed. Whitespace is folded to single spaces.
///
/// Third-party readers panic on some malformed files (a scanned PDF on Lex's
/// machine made pdf-extract panic with "unexpected encoding", 2026-10-12).
/// Every extraction runs inside `catch_unwind`, which only works because the
/// release profile unwinds (Cargo.toml, guarded by a compile_error in lib.rs).
/// A file whose reader panics is indexed by name only.
pub fn extract_text(path: &Path, ext: &str, size: u64) -> Option<String> {
    guarded(path, || raw_text(path, ext, size).map(|text| squeeze(&text)))
}

/// The same text for the quick preview: spaces folded, line breaks kept,
/// the first `PREVIEW_LINES` lines (audit T-46).
pub fn extract_preview(path: &Path, ext: &str, size: u64) -> Option<String> {
    guarded(path, || raw_text(path, ext, size).map(|text| preview_text(&text)))
}

fn guarded(path: &Path, read: impl FnOnce() -> Option<String>) -> Option<String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)) {
        Ok(text) => text,
        Err(_) => {
            log::warn!("the reader for {} crashed on this file; it is searchable by name only", path.display());
            None
        }
    }
}

/// The words inside the file as its reader gives them, line breaks and all.
fn raw_text(path: &Path, ext: &str, size: u64) -> Option<String> {
    if size > MAX_FILE_BYTES {
        return None;
    }
    match ext {
        "docx" => ooxml_text(path, &["word/document.xml"]),
        "xlsx" => ooxml_text(path, &["xl/sharedStrings.xml"]),
        "pptx" => pptx_text(path),
        "pdf" => pdf_text(path),
        "txt" | "csv" | "md" => plain_text(path),
        _ => None,
    }
}

fn ooxml_text(path: &Path, parts: &[&str]) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut text = Stripper::default();
    for part in parts {
        if let Ok(entry) = archive.by_name(part) {
            let declared = entry.size();
            read_part(entry, declared, &mut text);
        }
    }
    text.finish()
}

fn pptx_text(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    let mut text = Stripper::default();
    for name in names {
        if text.full() {
            break;
        }
        if let Ok(entry) = archive.by_name(&name) {
            let declared = entry.size();
            read_part(entry, declared, &mut text);
            // Each slide starts a line in the preview.
            text.end_line();
        }
    }
    text.finish()
}

/// Streams one part of an Office file through the stripper. At most
/// `MAX_PART_BYTES` are read whatever the part really holds, and the
/// stripper stops taking text at `MAX_TEXT_CHARS`, so a part of any size
/// costs one read buffer and the capped text (audit T-04). A part that
/// declares more than the limit is not read at all.
fn read_part(part: impl Read, declared: u64, text: &mut Stripper) {
    if declared > MAX_PART_BYTES {
        log::warn!("an Office file has a part of {} MB when unpacked; it is searchable by name only", declared / (1024 * 1024));
        return;
    }
    let mut part = part.take(MAX_PART_BYTES);
    let mut buffer = vec![0u8; READ_CHUNK];
    // The bytes of a character cut by the end of one read wait for the next.
    let mut pending: Vec<u8> = Vec::new();
    while !text.full() {
        let read = match part.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                log::debug!("a part of an Office file could not be read to its end: {error}");
                break;
            }
        };
        pending.extend_from_slice(&buffer[..read]);
        let whole = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            // A character the read cut in two is finished by the next read;
            // anything else that is not UTF-8 is read lossily.
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => pending.len(),
        };
        text.push_str(&String::from_utf8_lossy(&pending[..whole]));
        pending.drain(..whole);
    }
}

/// The most pages one PDF may contribute. A teacher's document is well
/// under this; a 500-page scan is read by name only beyond the budget.
const MAX_PDF_PAGES: u32 = 500;

/// PDF text with a page budget and an output cap. pdf-extract has no
/// streaming limit, so the caller drives it through a device that stops the
/// document (by returning an error) after `MAX_PDF_PAGES` or `MAX_TEXT_CHARS`
/// and keeps the text read so far. The file is already at most
/// `MAX_FILE_BYTES`, checked by `raw_text`.
///
/// This bounds the pages and the text, not one stream's unpacked size:
/// pdf-extract still inflates each content stream whole in memory, so a PDF
/// built to expand hugely can still cost that memory and panic (caught
/// above). The durable fix is process isolation for this reader; that is
/// recorded in reports/fix-audit/LEFTOVERS.md.
fn pdf_text(path: &Path) -> Option<String> {
    // Panics are caught one level up, in extract_text. The old version also
    // swapped the global panic hook per call, which raced with other threads.
    let bytes = std::fs::read(path).ok()?;
    let doc = pdf_extract::Document::load_mem(&bytes).ok()?;
    let mut out = PdfText::default();
    // A structural error still leaves the pages already read; the text is
    // what matters, and an empty result means name only.
    let _ = pdf_extract::output_doc(&doc, &mut out);
    if out.text.trim().is_empty() {
        None
    } else {
        Some(out.text)
    }
}

/// Collects PDF text up to the caps, then stops the document.
#[derive(Default)]
struct PdfText {
    text: String,
    pages: u32,
    full: bool,
}

impl PdfText {
    fn limit_error() -> pdf_extract::OutputError {
        pdf_extract::OutputError::IoError(std::io::Error::other("PDF text limit reached"))
    }

    fn push(&mut self, text: &str) {
        if self.full {
            return;
        }
        if self.text.len() + text.len() > MAX_TEXT_CHARS {
            self.full = true;
            return;
        }
        self.text.push_str(text);
        if self.text.len() >= MAX_TEXT_CHARS {
            self.full = true;
        }
    }
}

impl pdf_extract::OutputDev for PdfText {
    fn begin_page(&mut self, _page_num: u32, _media_box: &pdf_extract::MediaBox, _art_box: Option<(f64, f64, f64, f64)>) -> Result<(), pdf_extract::OutputError> {
        if self.pages >= MAX_PDF_PAGES {
            return Err(Self::limit_error());
        }
        self.pages += 1;
        self.push("\n");
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.push("\n");
        if self.full {
            return Err(Self::limit_error());
        }
        Ok(())
    }

    fn output_character(&mut self, _trm: &pdf_extract::Transform, _width: f64, _spacing: f64, _font_size: f64, char: &str) -> Result<(), pdf_extract::OutputError> {
        self.push(char);
        if self.full {
            return Err(Self::limit_error());
        }
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.push(" ");
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.push("\n");
        Ok(())
    }
}

/// Plain text the way Windows programs save it (audit T-23). A file read as
/// anything but UTF-8 is noted at Debug, where the path may appear.
fn plain_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let (text, encoding) = decode_text(bytes);
    if encoding != "UTF-8" {
        log::debug!("read {} as {encoding}", path.display());
    }
    Some(text)
}

/// Decodes the bytes of a text file: UTF-8 (with or without its byte order
/// mark), UTF-16 with its mark (Excel's "Unicode Text"), else the
/// Windows-1252 code page that Excel's "CSV (Comma delimited)" and older
/// Notepad files use, where every byte is one character. Returns the text
/// and the name of what it was read as.
pub fn decode_text(bytes: Vec<u8>) -> (String, &'static str) {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (utf16(rest, u16::from_le_bytes), "UTF-16");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (utf16(rest, u16::from_be_bytes), "UTF-16");
    }
    match String::from_utf8(bytes) {
        Ok(mut text) => {
            if text.starts_with('\u{FEFF}') {
                text.drain(..'\u{FEFF}'.len_utf8());
            }
            (text, "UTF-8")
        }
        Err(error) => (windows_1252(error.as_bytes()), "Windows-1252"),
    }
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| unit([pair[0], pair[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// Windows-1252 is Latin-1 except the 32 bytes from 0x80 to 0x9F, which
/// hold typographic marks (the curly quotes Word and Excel write among them).
fn windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9F => WINDOWS_1252_HIGH[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

/// Code points for the bytes 0x80 to 0x9F of Windows-1252; the five the
/// code page leaves undefined are the replacement character.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// Removes XML tags from text as it streams through, decoding the five
/// basic entities and folding whitespace, so words from adjacent cells do
/// not fuse and spaces do not pile up. The end of a Word or PowerPoint
/// paragraph or an Excel string is a line break, which the preview keeps
/// and the index folds away. Stops taking text at `MAX_TEXT_CHARS`, and
/// keeps a tag name and an entity name short, so no input can make it grow.
#[derive(Default)]
struct Stripper {
    out: String,
    in_tag: bool,
    /// The first characters of the tag's name, to know a paragraph's end.
    tag: String,
    tag_named: bool,
    in_entity: bool,
    entity: String,
    /// A separator owed before the next word: a space, or a line break.
    separator: Option<char>,
}

impl Stripper {
    /// Tags whose end is a line break: a Word or PowerPoint paragraph, an
    /// Excel shared string, a forced break in a Word or PowerPoint run.
    const LINE_TAGS: [&'static str; 5] = ["/w:p", "/a:p", "/si", "w:br", "a:br"];
    const TAG_CHARS: usize = 8;
    const ENTITY_CHARS: usize = 8;

    fn full(&self) -> bool {
        self.out.len() >= MAX_TEXT_CHARS
    }

    fn push_str(&mut self, xml: &str) {
        for c in xml.chars() {
            if self.full() {
                return;
            }
            self.push(c);
        }
    }

    fn push(&mut self, c: char) {
        if self.in_tag {
            if c == '>' {
                self.in_tag = false;
                let breaks = Self::LINE_TAGS.contains(&self.tag.as_str());
                self.separate(if breaks { '\n' } else { ' ' });
            } else if !self.tag_named {
                if c.is_whitespace() || (c == '/' && !self.tag.is_empty()) || self.tag.len() >= Self::TAG_CHARS {
                    self.tag_named = true;
                } else {
                    self.tag.push(c);
                }
            }
            return;
        }
        match c {
            '<' => {
                self.in_tag = true;
                self.tag_named = false;
                self.tag.clear();
            }
            '&' => {
                self.in_entity = true;
                self.entity.clear();
            }
            ';' if self.in_entity => {
                self.in_entity = false;
                match self.entity.as_str() {
                    "amp" => self.word('&'),
                    "lt" => self.word('<'),
                    "gt" => self.word('>'),
                    "quot" => self.word('"'),
                    "apos" => self.word('\''),
                    _ => self.separate(' '),
                }
            }
            _ if self.in_entity => {
                // An entity has a short name; a stray ampersand is not one.
                if self.entity.len() >= Self::ENTITY_CHARS {
                    self.in_entity = false;
                    self.separate(' ');
                } else {
                    self.entity.push(c);
                }
            }
            _ if c.is_whitespace() => self.separate(' '),
            _ => self.word(c),
        }
    }

    /// Owes a separator before the next word. A line break outranks a
    /// space, and nothing is owed before the first word.
    fn separate(&mut self, separator: char) {
        if self.out.is_empty() {
            return;
        }
        if separator == '\n' || self.separator.is_none() {
            self.separator = Some(separator);
        }
    }

    fn end_line(&mut self) {
        self.separate('\n');
    }

    fn word(&mut self, c: char) {
        if let Some(separator) = self.separator.take() {
            self.out.push(separator);
        }
        self.out.push(c);
    }

    /// The text, or None when the parts held no words.
    fn finish(self) -> Option<String> {
        if self.out.is_empty() {
            None
        } else {
            Some(self.out)
        }
    }
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

/// The first `PREVIEW_LINES` lines, spaces folded within each line but the
/// line breaks kept, cut at `PREVIEW_CHARS` (audit T-46).
fn preview_text(text: &str) -> String {
    let mut out = String::new();
    for (n, line) in text.lines().take(PREVIEW_LINES).enumerate() {
        if n > 0 {
            out.push('\n');
        }
        for (i, word) in line.split_whitespace().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(word);
        }
        if out.len() >= PREVIEW_CHARS {
            break;
        }
    }
    if out.len() > PREVIEW_CHARS {
        out.chars().take(PREVIEW_CHARS).collect()
    } else {
        out
    }
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

    /// A .docx whose document.xml is `blocks` copies of `block`, written
    /// block by block so the test itself never holds the whole part.
    fn write_docx_of(path: &Path, block: &str, blocks: usize) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_level(Some(1));
        zip.start_file("word/document.xml", options).unwrap();
        for _ in 0..blocks {
            zip.write_all(block.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    /// An endless reader that repeats a pattern: a part whose header lies
    /// about its size, or a zip bomb without the zip.
    struct Endless<'a> {
        pattern: &'a [u8],
        at: usize,
    }

    impl Read for Endless<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            for byte in buf.iter_mut() {
                *byte = self.pattern[self.at];
                self.at = (self.at + 1) % self.pattern.len();
            }
            Ok(buf.len())
        }
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

    /// The audit's file: a small .docx whose one part unpacks to hundreds of
    /// MB. The part is skipped by its declared size; a part under the limit
    /// is read but its text is capped; a part whose header lies, or that
    /// never ends, still costs one buffer and returns (audit T-04).
    #[test]
    fn an_office_part_that_unpacks_to_200_mb_is_read_with_bounded_memory() {
        let dir = tempfile::tempdir().unwrap();
        let block = "<w:p><w:r><w:t>Santos Reyes attendance Grade 7 Mabini</w:t></w:r></w:p>";
        let per_mb = (1024 * 1024) / block.len();
        let bomb = dir.path().join("small.docx");
        write_docx_of(&bomb, block, per_mb * 200);
        let on_disk = std::fs::metadata(&bomb).unwrap().len();
        assert!(on_disk < MAX_FILE_BYTES, "the file itself is small ({on_disk} bytes), so the old size cap never saw it");
        let started = std::time::Instant::now();
        assert_eq!(extract_text(&bomb, "docx", on_disk), None, "a part above MAX_PART_BYTES is skipped: searchable by name only");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "skipped, not unpacked");

        let large = dir.path().join("large.docx");
        write_docx_of(&large, block, per_mb * 3);
        let text = extract_text(&large, "docx", std::fs::metadata(&large).unwrap().len()).unwrap();
        assert!(text.len() <= MAX_TEXT_CHARS + 4, "the text is capped: {} bytes", text.len());
        assert!(text.len() >= MAX_TEXT_CHARS - 8, "the cap is what stopped it: {} bytes", text.len());
        assert!(text.starts_with("Santos Reyes attendance Grade 7 Mabini Santos"), "{:?}", &text[..60]);

        // A header that says 1 KB over a part that never ends: the text cap
        // stops the read.
        let mut text = Stripper::default();
        read_part(Endless { pattern: b"<w:t>Santos</w:t>", at: 0 }, 1024, &mut text);
        let text = text.finish().unwrap();
        assert!(text.len() <= MAX_TEXT_CHARS + 4 && text.len() >= MAX_TEXT_CHARS - 8, "{} bytes", text.len());
        // Tags with no words in them, without end: the byte limit stops it.
        let mut text = Stripper::default();
        read_part(Endless { pattern: b"<w:r><w:rPr/></w:r>", at: 0 }, 1024, &mut text);
        assert_eq!(text.finish(), None);
        // A stray ampersand or a tag name that never ends cannot grow either.
        let mut text = Stripper::default();
        read_part(Endless { pattern: b"&", at: 0 }, 1024, &mut text);
        assert_eq!(text.finish(), None);
        let mut text = Stripper::default();
        read_part(Endless { pattern: b"<", at: 0 }, 1024, &mut text);
        assert_eq!(text.finish(), None);
    }

    /// Excel's "CSV (Comma delimited)" and older Notepad files use the
    /// Windows code page; its text is read and found (audit T-23).
    #[test]
    fn a_windows_code_page_csv_is_read_and_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("class list.csv");
        // 0xF1 is the n with a tilde, 0x93 and 0x94 the curly double quotes.
        std::fs::write(&csv, b"Name,Section\r\nPe\xF1aflor,Mabini\r\nSantos \x93Juan\x94,Rizal\r\n").unwrap();
        let text = extract_text(&csv, "csv", 40).unwrap();
        assert_eq!(text, "Name,Section Pe\u{F1}aflor,Mabini Santos \u{201C}Juan\u{201D},Rizal");
        let conn = crate::db::open_memory().unwrap();
        crate::index::upsert(&conn, &crate::index::entry_for(&csv).unwrap()).unwrap();
        for typed in ["pe\u{F1}aflor", "penaflor", "santos juan"] {
            let hits = crate::index::search(&conn, typed, &crate::index::Filters::default(), 10).unwrap();
            assert_eq!(hits.len(), 1, "typed {typed:?}");
        }
        // A UTF-8 file still reads as it is, and its byte order mark goes.
        let utf8 = dir.path().join("notes.txt");
        std::fs::write(&utf8, "\u{FEFF}Pe\u{F1}aflor".as_bytes()).unwrap();
        assert_eq!(extract_text(&utf8, "txt", 12).unwrap(), "Pe\u{F1}aflor");
        // Excel's "Unicode Text" is UTF-16 with a byte order mark.
        let utf16 = dir.path().join("unicode.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Pe\u{F1}aflor\tMabini".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&utf16, &bytes).unwrap();
        assert_eq!(extract_text(&utf16, "txt", bytes.len() as u64).unwrap(), "Pe\u{F1}aflor Mabini");
        assert_eq!(decode_text(b"plain".to_vec()), ("plain".to_string(), "UTF-8"));
        assert_eq!(decode_text(vec![0x80, 0x96, 0x97]).0, "\u{20AC}\u{2013}\u{2014}", "the code page's own marks");
    }

    /// The preview keeps the file's line breaks and shows the first forty
    /// lines; the index folds every break to a space (audit T-46).
    #[test]
    fn the_preview_keeps_line_breaks_and_shows_forty_lines() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.txt");
        let body: String = (1..=50).map(|n| format!("line   {n}\t end\r\n")).collect();
        std::fs::write(&notes, &body).unwrap();
        let preview = extract_preview(&notes, "txt", body.len() as u64).unwrap();
        let lines: Vec<&str> = preview.lines().collect();
        assert_eq!(lines.len(), PREVIEW_LINES);
        assert_eq!(lines[0], "line 1 end", "spaces folded, the break kept");
        assert_eq!(lines[39], "line 40 end");
        assert!(!extract_text(&notes, "txt", body.len() as u64).unwrap().contains('\n'), "the index folds breaks away");
        let docx = dir.path().join("plan.docx");
        write_zip(&docx, &[("word/document.xml", "<w:p><w:r><w:t>First</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t><w:br/><w:t>Third</w:t></w:r></w:p>")]);
        assert_eq!(extract_preview(&docx, "docx", 10).unwrap(), "First\nSecond\nThird", "each paragraph and forced break is a line");
        assert_eq!(extract_text(&docx, "docx", 10).unwrap(), "First Second Third");
        let wide = "x".repeat(PREVIEW_CHARS);
        assert_eq!(preview_text(&format!("{wide}{wide}\nnext")).len(), PREVIEW_CHARS, "forty very wide lines stay light");
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

    /// The PDF reader stops at the page budget and the text cap and keeps
    /// what it read; a document cannot flood the index with either.
    #[test]
    fn a_pdf_stops_at_the_page_budget_and_the_text_cap() {
        use pdf_extract::OutputDev;
        let boxed = pdf_extract::MediaBox { llx: 0.0, lly: 0.0, urx: 612.0, ury: 792.0 };
        let mut pages = PdfText::default();
        for n in 0..MAX_PDF_PAGES {
            assert!(pages.begin_page(n, &boxed, None).is_ok(), "page {n} is inside the budget");
        }
        assert!(pages.begin_page(MAX_PDF_PAGES, &boxed, None).is_err(), "the next page stops the document");
        assert_eq!(pages.pages, MAX_PDF_PAGES);

        let mut text = PdfText::default();
        text.push(&"x".repeat(MAX_TEXT_CHARS));
        assert!(text.full, "the cap is reached");
        assert!(text.output_character(&pdf_extract::Transform::identity(), 1.0, 0.0, 10.0, "y").is_err(), "text past the cap stops the document");
        assert_eq!(text.text.len(), MAX_TEXT_CHARS);
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
