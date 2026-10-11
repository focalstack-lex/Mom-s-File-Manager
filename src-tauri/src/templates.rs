//! The Templates shelf (spec 5.6): master files that are copied, never edited.

use crate::error::{DeskError, ErrorCode, Result};
use crate::index;
use crate::ops;
use crate::paths::{self, Roots};
use crate::sorter;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub missing: bool,
}

pub fn list(conn: &Connection) -> Result<Vec<Template>> {
    let mut stmt = conn.prepare("SELECT id, path, name FROM templates ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(1)?;
        Ok(Template { id: r.get(0)?, missing: !Path::new(&path).is_file(), path, name: r.get(2)? })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<Template>, _>>()?)
}

pub fn add(conn: &Connection, roots: &Roots, path: &Path) -> Result<Template> {
    let path = roots.confine(path)?;
    if !path.is_file() {
        return Err(DeskError::new(ErrorCode::NotFound, "That file could not be found.").with_path(paths::display(&path)));
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    conn.execute(
        "INSERT OR IGNORE INTO templates(path, name, created_at) VALUES(?1, ?2, ?3)",
        params![path.to_string_lossy().into_owned(), name, crate::db::now()],
    )?;
    let id: i64 = conn.query_row("SELECT id FROM templates WHERE path = ?1", [path.to_string_lossy().into_owned()], |r| r.get(0))?;
    Ok(Template { id, path: path.to_string_lossy().into_owned(), name, missing: false })
}

pub fn remove(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM templates WHERE id = ?1", [id])?;
    Ok(())
}

/// Copies the master into `School Files\<Type>\<YYYY-MM>` as
/// `<name> <YYYY-MM-DD>.<ext>` and returns the copy's path.
pub fn use_template(conn: &Connection, roots: &Roots, id: i64) -> Result<PathBuf> {
    let (path, name): (String, String) = conn
        .query_row("SELECT path, name FROM templates WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| DeskError::new(ErrorCode::NotFound, "That template is no longer on the shelf."))?;
    let master = roots.confine(Path::new(&path))?;
    if !master.is_file() {
        return Err(DeskError::new(ErrorCode::NotFound, "The template file has been moved or deleted.").with_path(path));
    }
    let ext = sorter::extension(&master);
    let now = chrono::Local::now();
    let dir = sorter::destination(&roots.school_files()?, sorter::classify(&ext), now);
    std::fs::create_dir_all(&dir)?;
    let dotted = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let copy = sorter::unique_path(&dir.join(format!("{name} {}{dotted}", now.format("%Y-%m-%d"))));
    let op = ops_record_copy(conn, &master, &copy)?;
    let result = std::fs::copy(&master, &copy).map(|_| copy.clone()).map_err(DeskError::from);
    ops_finish(conn, op, &result)?;
    let copy = result?;
    index::refresh_path(conn, &copy)?;
    Ok(copy)
}

fn ops_record_copy(conn: &Connection, src: &Path, dst: &Path) -> Result<i64> {
    conn.execute(
        "INSERT INTO ops(kind, src, dst, status, automatic, created_at) VALUES('copy', ?1, ?2, 'pending', 0, ?3)",
        params![src.to_string_lossy().into_owned(), dst.to_string_lossy().into_owned(), crate::db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

fn ops_finish(conn: &Connection, id: i64, result: &Result<PathBuf>) -> Result<()> {
    match result {
        Ok(_) => conn.execute("UPDATE ops SET status = 'done' WHERE id = ?1", [id])?,
        Err(e) => conn.execute("UPDATE ops SET status = 'failed', message = ?2 WHERE id = ?1", params![id, e.message])?,
    };
    Ok(())
}

// Keep the ops module's undo aware of copies: it recycles the copy (ops::undo).
#[allow(dead_code)]
fn _uses_ops() -> ops::OpKind {
    ops::OpKind::Copy
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn template_copies_are_dated_and_the_master_stays() {
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        let roots = Roots::explicit(dir.path().join("Desktop"), documents.clone(), dir.path().join("Downloads"));
        let conn = db::open_memory().unwrap();
        let master = documents.join("Lesson Plan.docx");
        std::fs::write(&master, b"master").unwrap();
        let template = add(&conn, &roots, &master).unwrap();
        assert_eq!(template.name, "Lesson Plan");
        let copy = use_template(&conn, &roots, template.id).unwrap();
        assert!(copy.to_string_lossy().contains("School Files"));
        assert!(copy.file_name().unwrap().to_string_lossy().starts_with("Lesson Plan 20"));
        assert_eq!(std::fs::read(&master).unwrap(), b"master");
        let second = use_template(&conn, &roots, template.id).unwrap();
        assert_ne!(copy, second);
        assert_eq!(list(&conn).unwrap().len(), 1);
        remove(&conn, template.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}
