//! Safe display of untrusted source/entry/server text. Source and entry
//! text are untrusted terminal content: this module renders C0/C1
//! controls other than an intended line break or tab as a visible
//! placeholder, so an ESC/OSC sequence embedded in diary text or a
//! server-supplied name can never reach the real terminal backend.
//!
//! This only affects what is *displayed*; callers must never use the
//! sanitized output as the stored value (autosave, confirmation, digests)
//! — the architecture doc requires preserving the exact value and only
//! changing how it renders.

const PLACEHOLDER: char = '\u{FFFD}';

/// Returns a copy of `text` safe to hand to a terminal-rendering widget:
/// every C0 control (U+0000–U+001F, excluding `\n` and `\t`) and C1
/// control (U+007F, U+0080–U+009F) becomes `PLACEHOLDER`. Everything else,
/// including ordinary Unicode text and wide characters, passes through
/// unchanged.
#[must_use]
pub fn sanitize_for_display(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if is_unsafe_control(ch) {
                PLACEHOLDER
            } else {
                ch
            }
        })
        .collect()
}

fn is_unsafe_control(ch: char) -> bool {
    if ch == '\n' || ch == '\t' {
        return false;
    }
    let code = ch as u32;
    (code <= 0x1F) || code == 0x7F || (0x80..=0x9F).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_and_tab_pass_through_unchanged() {
        assert_eq!(sanitize_for_display("a\nb\tc"), "a\nb\tc");
    }

    #[test]
    fn an_escape_sequence_is_replaced_character_by_character() {
        // ESC (U+001B) '[' '2' 'J' would clear the screen if it reached
        // the real terminal unescaped.
        let input = "\u{1B}[2J";
        let sanitized = sanitize_for_display(input);
        assert!(!sanitized.contains('\u{1B}'));
        assert_eq!(sanitized, "\u{FFFD}[2J");
    }

    #[test]
    fn c1_controls_are_replaced() {
        assert_eq!(sanitize_for_display("a\u{0085}b"), "a\u{FFFD}b");
        assert_eq!(sanitize_for_display("a\u{009B}b"), "a\u{FFFD}b");
    }

    #[test]
    fn delete_is_replaced() {
        assert_eq!(sanitize_for_display("a\u{7F}b"), "a\u{FFFD}b");
    }

    #[test]
    fn ordinary_and_wide_text_is_unaffected() {
        let text = "hello 일기 쓰는 중";
        assert_eq!(sanitize_for_display(text), text);
    }

    #[test]
    fn sanitizing_never_changes_the_length_in_chars() {
        let input = "a\u{1B}b\u{7F}c";
        assert_eq!(
            sanitize_for_display(input).chars().count(),
            input.chars().count(),
            "sanitization must replace one-for-one, never insert or drop characters"
        );
    }
}
