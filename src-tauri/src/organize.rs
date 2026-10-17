//! Organize now (spec 5.11): the loose files on the Desktop and in Downloads
//! go into folders Tote creates, by grade and subject guessed from the file
//! name, else by type and month. She sees the whole plan first, and one Undo
//! reverses the run (`ops::undo_batch`).

use crate::error::Result;
use crate::index;
use crate::ops;
use crate::paths::{self, Roots};
use crate::sorter;
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One subject folder and the words in a file name that point to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectRule {
    pub folder: String,
    pub words: Vec<String>,
}

/// The standard list for a DepEd teacher; editable in Settings. Order
/// matters: the first rule whose word appears in the name wins.
pub fn default_rules() -> Vec<SubjectRule> {
    let rule = |folder: &str, words: &[&str]| SubjectRule { folder: folder.to_string(), words: words.iter().map(|w| w.to_string()).collect() };
    vec![
        rule("DepEd Forms", &["sf1", "sf2", "sf3", "sf4", "sf5", "sf6", "sf7", "sf8", "sf9", "sf10", "sf 1", "sf 2", "sf 3", "sf 4", "sf 5", "sf 6", "sf 7", "sf 8", "sf 9", "sf 10", "school form", "form 137", "form 138"]),
        rule("Math", &["math", "mathematics"]),
        rule("Science", &["science", "sci", "biology", "chemistry", "physics"]),
        rule("English", &["english", "eng", "reading", "grammar", "literature"]),
        rule("Filipino", &["filipino", "fil", "pagbasa", "wika"]),
        rule("Araling Panlipunan", &["araling panlipunan", "ap", "history", "social studies"]),
        rule("MAPEH", &["mapeh", "music", "arts", "pe", "health"]),
        rule("ESP", &["esp", "edukasyon sa pagpapakatao", "values", "gmrc"]),
        rule("TLE", &["tle", "tve", "ict", "home economics", "agriculture"]),
    ]
}

/// The rules in effect: hers when she has set any, the standard list otherwise.
pub fn effective_rules(saved: &[SubjectRule]) -> Vec<SubjectRule> {
    let cleaned: Vec<SubjectRule> = saved
        .iter()
        .filter(|r| !r.folder.trim().is_empty())
        .map(|r| SubjectRule { folder: r.folder.trim().to_string(), words: r.words.iter().map(|w| normalize(w)).filter(|w| !w.is_empty()).collect() })
        .filter(|r| !r.words.is_empty())
        .collect();
    if cleaned.is_empty() {
        default_rules()
    } else {
        cleaned
    }
}

/// Lowercase words separated by single spaces: "Grade_7-SCIENCE quiz" becomes "grade 7 science quiz".
pub fn normalize(text: &str) -> String {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// "Grade 7", "Kindergarten", or None, from a file name.
pub fn grade_of(stem: &str) -> Option<String> {
    let norm = normalize(stem);
    let tokens: Vec<&str> = norm.split(' ').collect();
    let level = |text: &str| text.parse::<u8>().ok().filter(|n| (1..=12).contains(n));
    for (i, token) in tokens.iter().enumerate() {
        if *token == "kinder" || *token == "kindergarten" {
            return Some("Kindergarten".to_string());
        }
        for prefix in ["grade", "baitang", "gr", "g"] {
            if *token == prefix {
                if let Some(n) = tokens.get(i + 1).and_then(|next| level(next)) {
                    return Some(format!("Grade {n}"));
                }
            } else if let Some(rest) = token.strip_prefix(prefix) {
                if let Some(n) = level(rest) {
                    return Some(format!("Grade {n}"));
                }
            }
        }
    }
    None
}

/// "Math 7", "AP 8", "English 10": DepEd names often put the grade right
/// after the subject. The number must follow the whole matched word, so
/// "SF 9 Report" (a form number) is not read as Grade 9.
pub fn grade_after_subject(stem: &str, rules: &[SubjectRule]) -> Option<String> {
    let norm = normalize(stem);
    let tokens: Vec<&str> = norm.split(' ').collect();
    for rule in rules {
        for word in &rule.words {
            let phrase = normalize(word);
            let parts: Vec<&str> = phrase.split(' ').filter(|p| !p.is_empty()).collect();
            if parts.is_empty() || parts.len() > tokens.len() {
                continue;
            }
            for start in 0..=(tokens.len() - parts.len()) {
                if tokens[start..start + parts.len()] == parts[..] {
                    let level = tokens.get(start + parts.len()).and_then(|t| t.parse::<u8>().ok()).filter(|n| (1..=12).contains(n));
                    if let Some(n) = level {
                        return Some(format!("Grade {n}"));
                    }
                }
            }
        }
    }
    None
}

/// The first subject whose word appears in the name, as whole words.
pub fn subject_of(stem: &str, rules: &[SubjectRule]) -> Option<String> {
    let padded = format!(" {} ", normalize(stem));
    rules
        .iter()
        .find(|rule| rule.words.iter().any(|w| !w.is_empty() && padded.contains(&format!(" {} ", normalize(w)))))
        .map(|rule| rule.folder.clone())
}

/// Shortcuts and system files belong where they are; installers and
/// archives are run or extracted once (the same rule as downloads).
pub fn is_movable(name: &str, ext: &str) -> bool {
    !matches!(ext, "lnk" | "url" | "ini" | "pif") && !sorter::stays_in_downloads(ext) && !name.eq_ignore_ascii_case("desktop.ini")
}

/// Where a file belongs, and why, in words she can read.
pub fn destination_for(school_files: &Path, name: &str, modified: i64, rules: &[SubjectRule]) -> (PathBuf, String) {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let grade = grade_of(&stem).or_else(|| grade_after_subject(&stem, rules));
    let subject = subject_of(&stem, rules);
    match (grade, subject) {
        (Some(g), Some(s)) => (school_files.join(&g).join(&s), format!("{g}, {s}, from the name")),
        (Some(g), None) => (school_files.join(&g), format!("{g}, from the name")),
        (None, Some(s)) => (school_files.join(&s), format!("{s}, from the name")),
        (None, None) => {
            let ext = sorter::extension(Path::new(name));
            let kind = sorter::classify(&ext);
            let when = Local.timestamp_opt(modified, 0).single().unwrap_or_else(Local::now);
            (sorter::destination(school_files, kind, when), "no grade or subject in the name, so by type and month".to_string())
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMove {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub kind: String,
    pub modified: i64,
    pub dest_dir: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCount {
    pub dir: String,
    pub count: usize,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub moves: Vec<PlannedMove>,
    pub folders: Vec<FolderCount>,
    /// Shortcuts, installers and archives left where they are.
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub batch: String,
    pub moved: usize,
    pub folders: usize,
    pub failed: Vec<String>,
}

/// The loose files directly on the Desktop and in Downloads, read from disk
/// so the plan is right even while the first scan is still running.
pub fn plan(roots: &Roots, rules: &[SubjectRule]) -> Result<Plan> {
    let school = roots.school_files()?;
    let mut moves = Vec::new();
    let mut skipped = 0;
    for folder in [roots.desktop.as_ref(), roots.downloads.as_ref()].into_iter().flatten() {
        let folder = paths::resolve(folder).unwrap_or_else(|_| folder.clone());
        let Ok(read) = std::fs::read_dir(&folder) else { continue };
        let mut entries: Vec<index::Entry> = read.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).filter_map(|e| index::entry_for(&e.path())).collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
        for entry in entries {
            let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = sorter::extension(&entry.path);
            if !is_movable(&name, &ext) {
                skipped += 1;
                continue;
            }
            let (dest, reason) = destination_for(&school, &name, entry.modified, rules);
            moves.push(PlannedMove {
                path: paths::display(&entry.path),
                name,
                kind: sorter::classify(&ext).as_str().to_string(),
                ext,
                modified: entry.modified,
                dest_dir: paths::display(&dest),
                reason,
            });
        }
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in &moves {
        *counts.entry(m.dest_dir.clone()).or_default() += 1;
    }
    let mut folders: Vec<FolderCount> = counts.into_iter().map(|(dir, count)| FolderCount { exists: Path::new(&dir).is_dir(), dir, count }).collect();
    folders.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.dir.cmp(&b.dir)));
    Ok(Plan { moves, folders, skipped })
}

/// Carries out the plan for the files she confirmed. Destinations are worked
/// out again here from the rules, never taken from the renderer. Each move
/// takes the lock only to record itself and to finish, never while its file
/// is copied, so searches keep answering during a long run (audit T-12).
pub fn run(core: &crate::state::Core, rules: &[SubjectRule], wanted: &[String]) -> Result<RunResult> {
    let roots = &core.roots;
    let plan = plan(roots, rules)?;
    let batch = ops::new_batch("organize");
    // The plan names files by their resolved paths; the confirmed list may
    // spell the same file differently (an 8.3 short name, a `.` segment), so
    // both sides are compared resolved.
    let wanted: Vec<std::path::PathBuf> = wanted.iter().map(|w| paths::resolve(Path::new(w)).unwrap_or_else(|_| std::path::PathBuf::from(w))).collect();
    let mut moved = 0;
    let mut failed = Vec::new();
    let mut folders = std::collections::HashSet::new();
    for m in plan.moves.iter().filter(|m| wanted.iter().any(|w| paths::same_path(w, Path::new(&m.path)))) {
        match ops::move_file_in_batch(core, roots, &core.placed, Path::new(&m.path), Path::new(&m.dest_dir), &batch) {
            Ok(_) => {
                moved += 1;
                folders.insert(m.dest_dir.clone());
            }
            Err(error) => {
                log::warn!("organize {}: {}", m.path, error.message);
                failed.push(m.name.clone());
            }
        }
    }
    log::info!("organize {batch}: moved {moved} files into {} folders, {} failed", folders.len(), failed.len());
    Ok(RunResult { batch, moved, folders: folders.len(), failed })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn grades_are_read_in_the_ways_teachers_write_them() {
        for name in ["Grade 7 Quiz", "grade7 quiz", "G7-Science", "Gr. 7 lesson", "gr7 notes", "Baitang 7 Pagsusulit", "DLL_Grade_7"] {
            assert_eq!(grade_of(name).as_deref(), Some("Grade 7"), "{name}");
        }
        assert_eq!(grade_of("Grade 12 STEM").as_deref(), Some("Grade 12"));
        assert_eq!(grade_of("Kinder Alphabet").as_deref(), Some("Kindergarten"));
        assert_eq!(grade_of("Group 7 photo"), None, "group is not gr");
        assert_eq!(grade_of("Grade 13 typo"), None);
        assert_eq!(grade_of("Lesson Plan"), None);
        assert_eq!(grade_of("g20 summit"), None);
    }

    #[test]
    fn a_number_right_after_the_subject_is_the_grade() {
        let rules = default_rules();
        let school = Path::new(r"C:\Docs\School Files");
        assert_eq!(destination_for(school, "AP 8 Reviewer.docx", 0, &rules).0, school.join("Grade 8").join("Araling Panlipunan"));
        assert_eq!(destination_for(school, "Math 7 drill sheet.pdf", 0, &rules).0, school.join("Grade 7").join("Math"));
        assert_eq!(destination_for(school, "English 10 module.docx", 0, &rules).0, school.join("Grade 10").join("English"));
        assert_eq!(destination_for(school, "SF 9 Report.xlsx", 0, &rules).0, school.join("DepEd Forms"), "a form number is not a grade");
        assert_eq!(grade_after_subject("Science 13 typo", &rules), None);
        assert_eq!(grade_after_subject("Quiz 2", &rules), None, "no subject, no grade");
    }

    #[test]
    fn subjects_match_whole_words_in_list_order() {
        let rules = default_rules();
        assert_eq!(subject_of("Grade 7 Science Quiz 2", &rules).as_deref(), Some("Science"));
        assert_eq!(subject_of("AP 8 Reviewer", &rules).as_deref(), Some("Araling Panlipunan"));
        assert_eq!(subject_of("SF9 Report Card", &rules).as_deref(), Some("DepEd Forms"));
        assert_eq!(subject_of("SF 2 attendance", &rules).as_deref(), Some("DepEd Forms"));
        assert_eq!(subject_of("Mathematics drill", &rules).as_deref(), Some("Math"));
        assert_eq!(subject_of("Capstone draft", &rules), None, "'ap' inside a word does not count");
        assert_eq!(subject_of("Lesson Plan", &rules), None);
    }

    #[test]
    fn edited_rules_replace_the_standard_list_and_blank_ones_are_ignored() {
        let mine = vec![SubjectRule { folder: " Robotics ".into(), words: vec!["Arduino".into(), " ".into()] }, SubjectRule { folder: "".into(), words: vec!["x".into()] }];
        let rules = effective_rules(&mine);
        assert_eq!(rules.len(), 1);
        assert_eq!(subject_of("arduino build log", &rules).as_deref(), Some("Robotics"));
        assert_eq!(subject_of("Science quiz", &rules), None);
        assert_eq!(effective_rules(&[]), default_rules());
    }

    #[test]
    fn destinations_and_what_never_moves() {
        let school = Path::new(r"C:\Docs\School Files");
        let rules = default_rules();
        assert_eq!(destination_for(school, "Grade 7 Science Quiz.docx", 0, &rules).0, school.join("Grade 7").join("Science"));
        assert_eq!(destination_for(school, "Grade 8 seat plan.xlsx", 0, &rules).0, school.join("Grade 8"));
        assert_eq!(destination_for(school, "English reviewer.pdf", 0, &rules).0, school.join("English"));
        let when = Local.with_ymd_and_hms(2026, 3, 5, 9, 0, 0).unwrap().timestamp();
        assert_eq!(destination_for(school, "random notes.txt", when, &rules).0, school.join("Other").join("2026-03"));
        assert!(!is_movable("Zoom.lnk", "lnk"));
        assert!(!is_movable("DepEd site.url", "url"));
        assert!(!is_movable("setup.exe", "exe"));
        assert!(!is_movable("desktop.ini", "ini"));
        assert!(is_movable("Quiz.docx", "docx"));
    }

    #[test]
    fn plan_reads_the_desktop_and_downloads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (desktop, documents, downloads) = (dir.path().join("Desktop"), dir.path().join("Documents"), dir.path().join("Downloads"));
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(desktop.join("Grade 7 Science Quiz.docx"), "x").unwrap();
        std::fs::write(desktop.join("Zoom.lnk"), "x").unwrap();
        std::fs::write(downloads.join("AP 8 Reviewer.pdf"), "x").unwrap();
        std::fs::write(downloads.join("setup.exe"), "x").unwrap();
        std::fs::create_dir_all(desktop.join("A folder")).unwrap();
        let roots = Roots::explicit(desktop, documents, downloads);
        let plan = plan(&roots, &default_rules()).unwrap();
        assert_eq!(plan.moves.len(), 2);
        assert_eq!(plan.skipped, 2, "the shortcut and the installer");
        assert_eq!(plan.folders.len(), 2);
        assert!(plan.folders.iter().all(|f| !f.exists));
        assert!(plan.moves.iter().any(|m| m.dest_dir.ends_with(r"Grade 7\Science") || m.dest_dir.ends_with("Grade 7/Science")));
    }

    #[test]
    fn a_run_is_one_batch_and_undoing_it_restores_everything() {
        let dir = tempfile::tempdir().unwrap();
        let (desktop, documents, downloads) = (dir.path().join("Desktop"), dir.path().join("Documents"), dir.path().join("Downloads"));
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(desktop.join("Grade 7 Science Quiz.docx"), "q").unwrap();
        std::fs::write(desktop.join("notes.txt"), "n").unwrap();
        std::fs::write(downloads.join("Not chosen.pdf"), "p").unwrap();
        let roots = Roots::explicit(desktop.clone(), documents.clone(), downloads.clone());
        let core = crate::state::Core {
            db: std::sync::Mutex::new(crate::db::open_memory().unwrap()),
            roots,
            status: std::sync::Mutex::new(Default::default()),
            recovered: false,
            rebuilt: None,
            data_dir: std::path::PathBuf::new(),
            tasks_restored: None,
            tasks_unavailable: false,
            placed: Default::default(),
        };
        let rules = default_rules();
        let wanted = vec![desktop.join("Grade 7 Science Quiz.docx").to_string_lossy().into_owned(), desktop.join("notes.txt").to_string_lossy().into_owned()];
        let result = run(&core, &rules, &wanted).unwrap();
        assert_eq!(result.moved, 2);
        assert_eq!(result.folders, 2);
        assert!(result.failed.is_empty());
        let school = documents.join("School Files");
        assert!(school.join("Grade 7").join("Science").join("Grade 7 Science Quiz.docx").is_file());
        assert!(downloads.join("Not chosen.pdf").is_file(), "only confirmed files move");
        let conn = core.db().unwrap();
        let undone = crate::ops::undo_batch(&conn, &core.roots, &core.placed, &result.batch).unwrap();
        assert_eq!(undone.undone, 2);
        assert!(desktop.join("Grade 7 Science Quiz.docx").is_file());
        assert!(desktop.join("notes.txt").is_file());
        assert!(!school.join("Grade 7").exists(), "folders the run created and left empty are removed");
    }

    /// The renderer's path and the plan's path can name the same file in
    /// different spellings (a `.` segment here; an 8.3 short name such as
    /// RUNNER~1 on the CI machine). The run must still move it.
    #[test]
    fn a_confirmed_file_moves_whatever_spelling_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let (desktop, documents, downloads) = (dir.path().join("Desktop"), dir.path().join("Documents"), dir.path().join("Downloads"));
        for d in [&desktop, &documents, &downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(desktop.join("Grade 7 Science Quiz.docx"), "q").unwrap();
        let core = crate::state::Core {
            db: std::sync::Mutex::new(crate::db::open_memory().unwrap()),
            roots: Roots::explicit(desktop.clone(), documents.clone(), downloads.clone()),
            status: std::sync::Mutex::new(Default::default()),
            recovered: false,
            rebuilt: None,
            data_dir: std::path::PathBuf::new(),
            tasks_restored: None,
            tasks_unavailable: false,
            placed: Default::default(),
        };
        let spelled = desktop.join(".").join("Grade 7 Science Quiz.docx");
        let result = run(&core, &default_rules(), &[spelled.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(result.moved, 1);
        assert!(documents.join("School Files").join("Grade 7").join("Science").join("Grade 7 Science Quiz.docx").is_file());
    }
}
