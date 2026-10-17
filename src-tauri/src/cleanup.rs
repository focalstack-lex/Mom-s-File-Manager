//! The three Clean up lists (spec 5.5). Reading only: every action she takes
//! on them goes through `ops`.

use crate::error::Result;
use crate::index::{self, FileRow, Group};
use crate::paths::Roots;
use crate::state::Db;
use rusqlite::Connection;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub file: FileRow,
    /// Where the type-and-month rule would put it.
    pub suggested_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    /// Groups whose `others` are copies of `latest` she may recycle.
    pub duplicates: Vec<Group>,
    /// Groups of different files that only share a name: `latest` and its
    /// `same_name` list, shown without tick boxes and never offered for the
    /// Recycle Bin (audit T-16).
    pub same_name: Vec<Group>,
    pub desktop: Vec<Suggestion>,
    pub old_downloads: Vec<Suggestion>,
}

pub const OLD_DOWNLOAD_DAYS: i64 = 60;

/// The lock is held for the index reads only; the content of same-size
/// files is compared after it is given back, so a search is not kept
/// waiting on the disk.
pub fn report(db: &impl Db, roots: &Roots, rules: &[crate::organize::SubjectRule]) -> Result<CleanupReport> {
    let (candidates, desktop, old_downloads) = {
        let conn = db.hold()?;
        (
            candidate_groups(&conn)?,
            loose_files(&conn, roots, roots.desktop.as_deref(), None, rules)?,
            loose_files(&conn, roots, roots.downloads.as_deref(), Some(crate::db::now() - OLD_DOWNLOAD_DAYS * 86_400), rules)?,
        )
    };
    let mut duplicates = Vec::new();
    let mut same_name = Vec::new();
    for (latest, members) in candidates {
        let (others, different): (Vec<FileRow>, Vec<FileRow>) = members.into_iter().partition(|m| is_copy(&latest, m));
        if !others.is_empty() {
            duplicates.push(Group { latest: latest.clone(), others, same_name: Vec::new() });
        }
        if !different.is_empty() {
            same_name.push(Group { latest, others: Vec::new(), same_name: different });
        }
    }
    Ok(CleanupReport { duplicates, same_name, desktop, old_downloads })
}

/// Every group key with more than one file, as its newest member and the rest.
fn candidate_groups(conn: &Connection) -> Result<Vec<(FileRow, Vec<FileRow>)>> {
    let keys: Vec<String> = {
        let mut stmt = conn.prepare("SELECT group_key FROM files GROUP BY group_key HAVING COUNT(*) > 1 ORDER BY MAX(modified) DESC LIMIT 200")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()?
    };
    let mut groups = Vec::new();
    for key in keys {
        // Newest first, so the first member is Latest.
        let mut members = index::group_members(conn, &key)?.into_iter();
        if let Some(latest) = members.next() {
            groups.push((latest, members.collect()));
        }
    }
    Ok(groups)
}

/// A copy she may recycle: its name carries a copy marker, or it is the
/// same size as `latest` and its content matches too. Same-name files of
/// another size, or of the same size with other content, are different
/// files (audit T-16).
fn is_copy(latest: &FileRow, member: &FileRow) -> bool {
    if crate::dupes::differ_by_copy_marker(&latest.stem(), &member.stem()) {
        return true;
    }
    member.size == latest.size && same_content(Path::new(&latest.path), Path::new(&member.path), latest.size)
}

/// How much of each end of two same-size files is compared.
const COMPARED_BYTES: u64 = 64 * 1024;

/// The first and last `COMPARED_BYTES` of two files of `size` bytes are the
/// same. A file that cannot be read counts as different, so nothing is
/// called a copy on a guess.
fn same_content(a: &Path, b: &Path, size: u64) -> bool {
    match (ends_of(a, size), ends_of(b, size)) {
        (Ok(x), Ok(y)) => x == y,
        (Err(error), _) | (_, Err(error)) => {
            log::debug!("clean up: could not compare {} and {} ({error})", a.display(), b.display());
            false
        }
    }
}

/// The start and the end of a file, at most `COMPARED_BYTES` each.
fn ends_of(path: &Path, size: u64) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() != size {
        // Changed since the index saw it: not known to be the same.
        return Err(std::io::Error::other("the size changed"));
    }
    let mut head = Vec::new();
    (&mut file).take(COMPARED_BYTES).read_to_end(&mut head)?;
    let mut tail = Vec::new();
    if size > COMPARED_BYTES {
        file.seek(SeekFrom::Start(size.saturating_sub(COMPARED_BYTES).max(COMPARED_BYTES)))?;
        file.take(COMPARED_BYTES).read_to_end(&mut tail)?;
    }
    Ok((head, tail))
}

fn loose_files(conn: &Connection, roots: &Roots, folder: Option<&std::path::Path>, older_than: Option<i64>, rules: &[crate::organize::SubjectRule]) -> Result<Vec<Suggestion>> {
    let Some(folder) = folder else { return Ok(Vec::new()) };
    let school = roots.school_files()?;
    // Rows store resolved paths (Roots::all), so compare against the resolved folder.
    let parent = crate::paths::resolve(folder).unwrap_or_else(|_| folder.to_path_buf()).to_string_lossy().into_owned();
    let sql = "SELECT path FROM files WHERE parent = ?1 AND (?2 IS NULL OR modified < ?2) ORDER BY modified DESC LIMIT 300";
    let paths: Vec<String> = {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![parent, older_than], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()?
    };
    let mut out = Vec::new();
    for path in paths {
        let Some(file) = index::by_path(conn, &path)? else { continue };
        // Shortcuts never move (a Desktop is mostly shortcuts); the home is the
        // same one Organize would choose, so the two never disagree.
        if !crate::organize::is_movable(&file.name, &file.ext) {
            continue;
        }
        let suggested_dir = crate::organize::destination_for(&school, &file.name, file.modified, rules).0.to_string_lossy().into_owned();
        out.push(Suggestion { file, suggested_dir });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn report_lists_duplicates_desktop_and_old_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = dir.path().join("Desktop");
        let documents = dir.path().join("Documents");
        let downloads = dir.path().join("Downloads");
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        let roots = Roots::explicit(desktop.clone(), documents, downloads.clone());
        let conn = db::open_memory().unwrap();
        std::fs::write(desktop.join("Plan.docx"), b"a").unwrap();
        std::fs::write(desktop.join("Plan (2).docx"), b"b").unwrap();
        std::fs::write(desktop.join("setup.exe"), b"b").unwrap();
        std::fs::write(downloads.join("old.pdf"), b"c").unwrap();
        for root in roots.all() {
            let mut entries = Vec::new();
            index::walk(&root, &mut entries);
            for e in &entries {
                index::upsert(&conn, e).unwrap();
            }
        }
        // Age the download in the index so it counts as old.
        conn.execute("UPDATE files SET modified = modified - 100 * 86400 WHERE name = 'old.pdf'", []).unwrap();
        let report = report(&conn, &roots, &crate::organize::default_rules()).unwrap();
        assert_eq!(report.duplicates.len(), 1);
        assert_eq!(report.duplicates[0].others.len(), 1);
        assert_eq!(report.desktop.len(), 2, "installer is skipped");
        assert!(report.desktop[0].suggested_dir.contains("School Files"));
        assert_eq!(report.old_downloads.len(), 1);
        assert!(report.old_downloads[0].suggested_dir.contains("PDF"));
    }

    /// Excel's and Word's default names in three folders are different
    /// documents unless their content matches (audit T-16).
    #[test]
    fn same_name_files_are_copies_only_when_their_content_matches() {
        let dir = tempfile::tempdir().unwrap();
        let (desktop, documents, downloads) = (dir.path().join("Desktop"), dir.path().join("Documents"), dir.path().join("Downloads"));
        let (rizal, bonifacio, pta, a, b, c) = (documents.join("Rizal"), documents.join("Bonifacio"), documents.join("PTA"), documents.join("A"), documents.join("B"), documents.join("C"));
        for d in [&desktop, &documents, &downloads, &rizal, &bonifacio, &pta, &a, &b, &c] {
            std::fs::create_dir_all(d).unwrap();
        }
        // Three different class lists, three sizes.
        std::fs::write(rizal.join("Book1.xlsx"), b"Grade 7 Rizal class list").unwrap();
        std::fs::write(bonifacio.join("Book1.xlsx"), b"Grade 8 Bonifacio list").unwrap();
        std::fs::write(pta.join("Book1.xlsx"), b"PTA collections").unwrap();
        // One size, two contents: a real copy and a different letter. The
        // files are large enough that the end of the file is compared too.
        let mut letter = vec![b'x'; 200 * 1024];
        std::fs::write(a.join("New Microsoft Word Document.docx"), &letter).unwrap();
        std::fs::write(b.join("New Microsoft Word Document.docx"), &letter).unwrap();
        *letter.last_mut().unwrap() = b'y';
        std::fs::write(c.join("New Microsoft Word Document.docx"), &letter).unwrap();
        let roots = Roots::explicit(desktop, documents, downloads);
        let conn = db::open_memory().unwrap();
        for root in roots.all() {
            let mut entries = Vec::new();
            index::walk(&root, &mut entries);
            for e in &entries {
                index::upsert(&conn, e).unwrap();
            }
        }
        let age = |folder: &Path, modified: i64| {
            let path = crate::paths::resolve(&folder.join("New Microsoft Word Document.docx")).unwrap();
            assert_eq!(conn.execute("UPDATE files SET modified = ?2 WHERE path = ?1", rusqlite::params![path.to_string_lossy().into_owned(), modified]).unwrap(), 1);
        };
        // C is the newest, so it is Latest and both others differ from it at the end.
        age(&a, 200);
        age(&b, 100);
        age(&c, 300);
        let report = report(&conn, &roots, &crate::organize::default_rules()).unwrap();
        assert!(report.duplicates.is_empty(), "nothing here is a copy of its Latest: {:?}", report.duplicates);
        assert_eq!(report.same_name.len(), 2);
        let book = report.same_name.iter().find(|g| g.latest.name == "Book1.xlsx").unwrap();
        assert_eq!(book.same_name.len(), 2);
        assert!(book.others.is_empty());

        // Now A is the newest: B has the same size and the same content, C does not.
        age(&a, 400);
        let report = super::report(&conn, &roots, &crate::organize::default_rules()).unwrap();
        assert_eq!(report.duplicates.len(), 1);
        let copies = &report.duplicates[0];
        assert!(copies.latest.path.contains(&format!("{}A{}", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR)));
        assert_eq!(copies.others.len(), 1);
        assert!(copies.others[0].path.contains(&format!("{}B{}", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR)));
        let letters = report.same_name.iter().find(|g| g.latest.name.starts_with("New")).unwrap();
        assert_eq!(letters.same_name.len(), 1);
        assert!(letters.same_name[0].path.contains(&format!("{}C{}", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR)));
    }
}
