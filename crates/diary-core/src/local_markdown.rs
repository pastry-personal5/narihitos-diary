use crate::{DiaryDay, WritingLanguage};

/// Canonical UTF-8 payload used by preview, digest and local publication.
pub fn render_local_markdown(day: &DiaryDay, language: WritingLanguage, body: &str) -> String {
    format!(
        "---\ndate: \"{}\"\nlanguage: \"{}\"\n---\n\n{}\n",
        day,
        language.as_str(),
        body.trim_end_matches('\n')
    )
}

/// The existing entry bytes followed by a dated separator and the new body.
///
/// The existing bytes are kept as they are, with only a final newline added
/// when missing. A blank line precedes `---` so it stays a thematic break and
/// never turns the last existing line into a heading.
#[must_use]
pub fn append_to_existing(existing: &str, body: &str, appended_at: &str) -> String {
    let body = body.trim_end_matches('\n');
    if existing.is_empty() {
        // Nothing to separate from, and a leading rule would read as front matter.
        return format!("*Appended {appended_at}*\n\n{body}\n");
    }
    let mut result = String::with_capacity(existing.len() + body.len() + 48);
    result.push_str(existing);
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result.push_str(&format!("\n---\n\n*Appended {appended_at}*\n\n{body}\n"));
    result
}

/// Name of the copy kept beside `target` before its content is replaced. It
/// never ends in `.md`, so it is not mistaken for a diary entry, and it is
/// derived from the replaced content so a replay finds the same backup.
#[must_use]
pub fn backup_file_name(target: &str, replaced: &crate::Digest) -> String {
    format!("{target}.bak-{}", &replaced.as_str()[..12])
}

#[cfg(test)]
mod append_tests {
    use super::*;

    #[test]
    fn append_keeps_the_existing_bytes_and_separates_with_a_thematic_break() {
        let existing = "---\ndate: \"2026-10-30\"\nlanguage: \"en-US\"\n---\n\nFirst.\n";
        let result = append_to_existing(existing, "Second.\n\n", "2026-10-30 21:14");
        assert!(result.starts_with(existing));
        assert_eq!(
            &result[existing.len()..],
            "\n---\n\n*Appended 2026-10-30 21:14*\n\nSecond.\n"
        );
    }

    #[test]
    fn a_missing_final_newline_is_added_and_a_blank_line_precedes_the_rule() {
        let result = append_to_existing("No newline", "New", "2026-10-30 21:14");
        assert_eq!(
            result,
            "No newline\n\n---\n\n*Appended 2026-10-30 21:14*\n\nNew\n"
        );
        // Without the blank line "No newline\n---" would be a heading.
        assert!(result.contains("\n\n---\n"));
    }

    #[test]
    fn an_empty_file_and_crlf_content_are_handled_without_rewriting_existing_bytes() {
        let crlf = "line one\r\nline two\r\n";
        assert!(append_to_existing(crlf, "x", "t").starts_with(crlf));
        assert_eq!(append_to_existing("", "x", "t"), "*Appended t*\n\nx\n");
    }

    #[test]
    fn the_backup_name_is_derived_from_the_replaced_content() {
        let digest = crate::Digest::of_bytes(b"old");
        let name = backup_file_name("/a/2026-10-30.md", &digest);
        assert_eq!(
            name,
            format!("/a/2026-10-30.md.bak-{}", &digest.as_str()[..12])
        );
        assert!(!name.ends_with(".md"));
    }
}
