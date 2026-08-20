//! Shared position-conversion helpers for service-side IDE features.
//!
//! Mirrors `sysml_lsp_server::utils::{position_to_offset, offset_to_position}`
//! but stays free of any tower-lsp dependency: callers pass plain `u32`
//! line/column values (UTF-16 code units, 0-indexed, per LSP convention) and
//! receive a byte offset (or vice-versa).
//!
//! Used by the `references`, `goto_definition`, `hover`, and `completion`
//! service modules — every position-sensitive read-only IDE query.

/// Convert (line, character) in UTF-16 code units to a byte offset.
///
/// Mirrors `sysml_lsp_server::utils::position_to_offset` so the byte offset
/// returned here lines up with what the LSP shell would have computed for the
/// same `(line, character)` pair.
pub fn position_to_offset(line: u32, character: u32, source: &str) -> usize {
    let mut offset = 0;
    for (current_line, line_text) in source.lines().enumerate() {
        if current_line as u32 == line {
            let mut char_count = 0u32;
            for (byte_idx, c) in line_text.char_indices() {
                if char_count >= character {
                    return offset + byte_idx;
                }
                char_count += c.len_utf16() as u32;
            }
            return offset + line_text.len();
        }
        offset += line_text.len() + 1;
    }
    source.len()
}

/// Convert a byte offset to (line, character) in UTF-16 code units.
///
/// Mirrors `sysml_lsp_server::utils::offset_to_position`. The character count
/// is in UTF-16 code units so positions emitted by service can be passed
/// directly back to LSP clients.
pub fn offset_to_line_col(offset: usize, source: &str) -> (u32, u32) {
    let mut clamped = offset.min(source.len());
    while clamped > 0 && !source.is_char_boundary(clamped) {
        clamped -= 1;
    }
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (idx, ch) in source.char_indices() {
        if idx >= clamped {
            let end = clamped.min(source.len());
            let start = line_start.min(end);
            let line_text = &source[start..end];
            let character: u32 = line_text.chars().map(|c| c.len_utf16() as u32).sum();
            return (line, character);
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + 1;
        }
    }
    let start = line_start.min(source.len());
    let line_text = &source[start..];
    let character: u32 = line_text.chars().map(|c| c.len_utf16() as u32).sum();
    (line, character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_to_offset_zero_origin() {
        let src = "abc\ndef\nghi";
        assert_eq!(position_to_offset(0, 0, src), 0);
        assert_eq!(position_to_offset(0, 3, src), 3);
        assert_eq!(position_to_offset(1, 0, src), 4);
        assert_eq!(position_to_offset(2, 2, src), 10);
    }

    #[test]
    fn offset_to_line_col_round_trip() {
        let src = "abc\ndef\nghi";
        for offset in 0..=src.len() {
            let (line, col) = offset_to_line_col(offset, src);
            let back = position_to_offset(line, col, src);
            assert_eq!(back, offset, "round-trip at offset={offset}");
        }
    }

    #[test]
    fn utf16_surrogate_pair_counts_as_two_units() {
        // 'a' (1 utf16) + '🦀' (2 utf16, 4 utf8 bytes)
        let src = "a🦀b";
        // After 'a', col = 1
        let (_, col_after_a) = offset_to_line_col(1, src);
        assert_eq!(col_after_a, 1);
        // After 🦀, col = 1 + 2 = 3, byte offset = 1 + 4 = 5
        let (_, col_after_crab) = offset_to_line_col(5, src);
        assert_eq!(col_after_crab, 3);
        // (line=0, col=3) → byte offset 5
        assert_eq!(position_to_offset(0, 3, src), 5);
    }

    #[test]
    fn offset_in_middle_of_multibyte_char_clamps_to_boundary() {
        let src = "🦀";
        let (_, col) = offset_to_line_col(2, src); // mid-codepoint
        // Should clamp to 0 and report col 0
        assert_eq!(col, 0);
    }
}
