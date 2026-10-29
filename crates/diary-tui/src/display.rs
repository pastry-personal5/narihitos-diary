//! Display-column helpers shared by adaptive widgets. Ratatui's own width
//! calculation is the authority, so UTF-8 byte length never drives clipping,
//! cursor placement, or hit testing.

use ratatui::text::Line;

#[must_use]
pub fn width(text: &str) -> usize {
    Line::from(text).width()
}

#[must_use]
pub fn truncate(text: &str, max_width: u16) -> String {
    let limit = usize::from(max_width);
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if width(&text[..next]) > limit {
            break;
        }
        end = next;
    }
    text[..end].to_owned()
}

/// Maps a terminal column to a UTF-8 boundary without ever returning the
/// middle of a wide scalar. This is used for cursor placement and is also the
/// primitive a future pointer-capable frontend can use for hit testing.
#[must_use]
pub fn byte_index_at_column(text: &str, column: u16) -> usize {
    let target = usize::from(column);
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if width(&text[..next]) > target {
            return index;
        }
        end = next;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_korean_syllables_as_two_columns() {
        assert_eq!(truncate("ab한글cd", 6), "ab한글");
        assert_eq!(width(&truncate("ab한글cd", 6)), 6);
    }

    #[test]
    fn column_hit_testing_never_returns_the_middle_of_utf8_or_a_wide_cell() {
        let text = "a한b";
        assert_eq!(byte_index_at_column(text, 1), 1);
        assert_eq!(byte_index_at_column(text, 2), 1);
        assert_eq!(byte_index_at_column(text, 3), 4);
        assert!(text.is_char_boundary(byte_index_at_column(text, 2)));
    }
}
