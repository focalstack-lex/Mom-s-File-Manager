//! Grouping near-duplicate copies (spec 5.3): `Lesson Plan (3).docx`,
//! `lesson plan final_final.docx` and `Copy of Lesson Plan.docx` share one key.

/// A key shared by every copy of the same document.
pub fn group_key(stem: &str, ext: &str) -> String {
    let mut text = stem.to_lowercase();
    // Strip "(n)" and bracketed numbers anywhere.
    text = strip_parenthesized_numbers(&text);
    // Split into words on separators and drop the noise words.
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !is_noise(w))
        .collect();
    if words.is_empty() {
        return format!("{}|{}", stem.to_lowercase(), ext.to_lowercase());
    }
    format!("{}|{}", words.join(" "), ext.to_lowercase())
}

/// True when two names that share a `group_key` differ, which can only be
/// by a copy marker: "Lesson Plan (2)", "Copy of Lesson Plan", "Lesson Plan
/// final". Two names that read the same once case and separators are set
/// aside ("Book1.xlsx" in three folders, "New Microsoft Word Document")
/// say nothing about the content, so the caller decides by size or content
/// instead (audit T-16).
pub fn differ_by_copy_marker(a_stem: &str, b_stem: &str) -> bool {
    name_form(a_stem) != name_form(b_stem)
}

/// A stem's words in lower case, every marker kept.
fn name_form(stem: &str) -> String {
    let lower = stem.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn is_noise(word: &str) -> bool {
    if matches!(word, "copy" | "of" | "final" | "revised" | "new" | "latest" | "edited" | "updated") {
        return true;
    }
    // v2, v10, and bare numbers that came from "(2)" style suffixes.
    if let Some(rest) = word.strip_prefix('v') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

fn strip_parenthesized_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '(' {
            let mut inner = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == ')' {
                    closed = true;
                    break;
                }
                inner.push(d);
            }
            if closed && !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            out.push('(');
            out.push_str(&inner);
            if closed {
                out.push(')');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn copies_share_a_key() {
        let base = group_key("Lesson Plan", "docx");
        assert_eq!(group_key("Lesson Plan (3)", "docx"), base);
        assert_eq!(group_key("lesson plan final_final", "DOCX"), base);
        assert_eq!(group_key("Copy of Lesson Plan", "docx"), base);
        assert_eq!(group_key("Lesson-Plan v2", "docx"), base);
        assert_eq!(group_key("Lesson Plan - revised", "docx"), base);
    }

    #[test]
    fn different_documents_differ() {
        assert_ne!(group_key("Lesson Plan", "docx"), group_key("Lesson Plan", "pdf"));
        assert_ne!(group_key("Grade 7 Quiz", "docx"), group_key("Grade 8 Quiz", "docx"));
    }

    #[test]
    fn only_a_copy_marker_makes_a_name_a_copy() {
        assert!(differ_by_copy_marker("Lesson Plan", "Lesson Plan (2)"));
        assert!(differ_by_copy_marker("Lesson Plan", "Copy of Lesson Plan"));
        assert!(differ_by_copy_marker("Lesson Plan", "lesson plan final"));
        assert!(!differ_by_copy_marker("Book1", "Book1"));
        assert!(!differ_by_copy_marker("New Microsoft Word Document", "New Microsoft Word Document"));
        assert!(!differ_by_copy_marker("Lesson Plan", "lesson-plan"), "case and separators are not markers");
    }

    #[test]
    fn numbers_inside_names_are_kept() {
        assert_ne!(group_key("Quiz 1", "docx"), group_key("Quiz 2", "docx"));
        assert_eq!(group_key("Quiz 1 (2)", "docx"), group_key("Quiz 1", "docx"));
    }
}
