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
