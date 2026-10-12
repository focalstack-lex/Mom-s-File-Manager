//! The three Clean up lists (spec 5.5). Reading only: every action she takes
//! on them goes through `ops`.

use crate::error::Result;
use crate::index::{self, FileRow, Group};
use crate::paths::Roots;
use rusqlite::Connection;
use serde::Serialize;

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
    pub duplicates: Vec<Group>,
    pub desktop: Vec<Suggestion>,
    pub old_downloads: Vec<Suggestion>,
}

pub const OLD_DOWNLOAD_DAYS: i64 = 60;

pub fn report(conn: &Connection, roots: &Roots, rules: &[crate::organize::SubjectRule]) -> Result<CleanupReport> {
    Ok(CleanupReport {
        duplicates: duplicates(conn)?,
        desktop: loose_files(conn, roots, roots.desktop.as_deref(), None, rules)?,
        old_downloads: loose_files(conn, roots, roots.downloads.as_deref(), Some(crate::db::now() - OLD_DOWNLOAD_DAYS * 86_400), rules)?,
    })
}

fn duplicates(conn: &Connection) -> Result<Vec<Group>> {
    let keys: Vec<String> = {
        let mut stmt = conn.prepare("SELECT group_key FROM files GROUP BY group_key HAVING COUNT(*) > 1 ORDER BY MAX(modified) DESC LIMIT 200")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()?
    };
    let mut groups = Vec::new();
    for key in keys {
        let members = index::group_members(conn, &key)?;
        let mut grouped = index::group(members);
        groups.append(&mut grouped);
    }
    Ok(groups)
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
}
