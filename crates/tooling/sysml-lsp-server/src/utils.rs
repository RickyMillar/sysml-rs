//! Utility functions for position/offset conversion, URI parsing, and completion scoring.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use std::path::PathBuf;

use tower_lsp::lsp_types::{Url, Position, Range};

#[cfg(test)]
use crate::lsp_types::Range as LspRange;

/// Convert an sysml-lsp Range to tower-lsp Range.
#[cfg(test)]
pub(crate) fn to_lsp_range(range: LspRange) -> Range {
    Range {
        start: Position {
            line: range.start.line,
            character: range.start.character,
        },
        end: Position {
            line: range.end.line,
            character: range.end.character,
        },
    }
}

/// Parse a URI string into a Url, handling both `file://` and plain paths.
pub(crate) fn parse_uri(uri: &str) -> Option<Url> {
    Url::parse(uri)
        .ok()
        .or_else(|| Url::from_file_path(uri).ok())
        .or_else(|| resolve_relative_library_path(uri))
}

fn resolve_relative_library_path(path: &str) -> Option<Url> {
    let relative = PathBuf::from(path);
    if relative.is_absolute() {
        return Url::from_file_path(relative).ok();
    }

    let config = crate::workspace::find_library_config()?;
    let mut candidates = vec![config.library_path.join(path)];
    for subdir in ["library.kernel", "library.systems", "library.domain"] {
        candidates.push(config.library_path.join(subdir).join(path));
    }

    for candidate in candidates {
        if candidate.exists() {
            if let Ok(canonical) = candidate.canonicalize() {
                if let Ok(url) = Url::from_file_path(canonical) {
                    return Some(url);
                }
            }
            if let Ok(url) = Url::from_file_path(&candidate) {
                return Some(url);
            }
        }
    }

    None
}

/// Convert LSP Position to byte offset in the source text.
pub(crate) fn position_to_offset(pos: &Position, source: &str) -> usize {
    let mut offset = 0;

    for (current_line, line) in source.lines().enumerate() {
        if current_line as u32 == pos.line {
            let mut char_count = 0u32;
            for (byte_idx, c) in line.char_indices() {
                if char_count >= pos.character {
                    return offset + byte_idx;
                }
                char_count += c.len_utf16() as u32;
            }
            return offset + line.len();
        }
        offset += line.len() + 1;
    }

    source.len()
}

pub(crate) fn clamp_to_char_boundary(source: &str, offset: usize) -> usize {
    let mut clamped = offset.min(source.len());
    while clamped > 0 && !source.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

/// Convert byte offset to LSP Position.
pub(crate) fn offset_to_position(offset: usize, source: &str) -> Position {
    let offset = clamp_to_char_boundary(source, offset);
    let mut line = 0u32;
    let mut line_start = 0;

    for (idx, ch) in source.char_indices() {
        if idx >= offset {
            let end = offset.min(source.len());
            let start = line_start.min(end);
            let line_text = &source[start..end];
            let character = line_text.chars().map(|c| c.len_utf16() as u32).sum();
            return Position { line, character };
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + 1;
        }
    }

    let start = line_start.min(source.len());
    let line_text = &source[start..];
    let character = line_text.chars().map(|c| c.len_utf16() as u32).sum();
    Position { line, character }
}

/// Precomputed line-start offsets for O(log n) `offset -> Position` lookups.
///
/// `offset_to_position` scans `char_indices()` from the start of the source on
/// every call — O(offset). Code that converts many offsets over the same
/// source (e.g. semantic-token generation, which converts 2 offsets per token)
/// pays O(tokens * file_len). Build a `LineIndex` once and call
/// [`LineIndex::position`] instead: a binary search for the line plus a scan of
/// the (short) line prefix for the UTF-16 character. Results are bit-identical
/// to [`offset_to_position`].
pub(crate) struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the first character of each line. Always starts with `0`.
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let mut line_starts = Vec::with_capacity(source.bytes().filter(|&b| b == b'\n').count() + 1);
        line_starts.push(0);
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    /// Convert a byte offset to an LSP `Position`. Bit-identical to
    /// [`offset_to_position`] but O(log lines + line length).
    pub(crate) fn position(&self, offset: usize) -> Position {
        let offset = clamp_to_char_boundary(self.source, offset);
        // Largest line_start <= offset. `line_starts[0] == 0 <= offset`, so
        // `partition_point` is always >= 1 and the subtraction can't underflow.
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let line_start = self.line_starts[line];
        // `line_start` is a char boundary (0 or just past a '\n') and
        // `line_start <= offset`, so this slice is valid.
        let line_text = &self.source[line_start..offset];
        let character = line_text.chars().map(|c| c.len_utf16() as u32).sum();
        Position {
            line: line as u32,
            character,
        }
    }
}

/// Score a completion match. Returns 0 if no match.
/// - 100: exact match
/// - 80: case-insensitive exact match
/// - 60: prefix match
/// - 40: case-insensitive prefix match
/// - 20: fuzzy (subsequence) match
pub(crate) fn score_completion(query: &str, label: &str) -> u32 {
    if query.is_empty() {
        return 50; // show everything when no query
    }
    if label == query {
        return 100;
    }
    let query_lower = query.to_lowercase();
    let label_lower = label.to_lowercase();
    if label_lower == query_lower {
        return 80;
    }
    if label.starts_with(query) {
        return 60;
    }
    if label_lower.starts_with(&query_lower) {
        return 40;
    }
    if fuzzy_match(&query_lower, &label_lower) {
        return 20;
    }
    0
}

/// Simple fuzzy matching: checks if all characters of the query appear in order
/// within the target string. This enables queries like "pdef" matching "PartDefinition".
pub(crate) fn fuzzy_match(query: &str, target: &str) -> bool {
    // Substring match is always accepted
    if target.contains(query) {
        return true;
    }
    // Subsequence match: all query chars appear in order in target
    let mut target_chars = target.chars();
    for qc in query.chars() {
        loop {
            match target_chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// Safely slice a string by byte offsets, returning `None` if out of bounds.
///
/// This is the **only** way span byte offsets should be used to index into
/// source text, because `doc.parsed` can lag behind `doc.content` (the parse
/// cache "may be from a previous version") making raw `&source[start..end]`
/// a panic risk.
pub(crate) fn safe_slice(source: &str, start: usize, end: usize) -> Option<&str> {
    if start <= end
        && end <= source.len()
        && source.is_char_boundary(start)
        && source.is_char_boundary(end)
    {
        Some(&source[start..end])
    } else {
        None
    }
}

/// Convert byte range to LSP Range.
pub(crate) fn range_to_lsp_range(start: usize, end: usize, source: &str) -> Range {
    let clamped_start = clamp_to_char_boundary(source, start);
    let clamped_end = clamp_to_char_boundary(source, end);
    Range {
        start: offset_to_position(clamped_start, source),
        end: offset_to_position(clamped_end, source),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn line_index_matches_offset_to_position() {
        // Multi-line source with a multi-byte / multi-UTF-16 char and a
        // trailing newline + empty last line. LineIndex must agree with the
        // scanning offset_to_position at every byte offset.
        let source = "part a;\nlet x = \u{2192}b;\n\npart c;\n";
        let index = LineIndex::new(source);
        for offset in 0..=source.len() {
            assert_eq!(
                index.position(offset),
                offset_to_position(offset, source),
                "mismatch at offset {offset}"
            );
        }
    }

    #[test]
    fn line_index_empty_source() {
        let source = "";
        let index = LineIndex::new(source);
        assert_eq!(index.position(0), offset_to_position(0, source));
        // Out-of-range offset clamps identically.
        assert_eq!(index.position(5), offset_to_position(5, source));
    }

    #[test]
    fn offset_to_position_non_char_boundary_is_safe() {
        let source = "aa\u{2192}bb\n";
        let arrow_idx = source.find('\u{2192}').expect("arrow should exist");
        let inside_arrow = arrow_idx + 1;
        let pos = offset_to_position(inside_arrow, source);

        // Non-boundary offsets are clamped to the previous char boundary.
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 2);
    }

    #[test]
    fn range_to_lsp_range_non_char_boundaries_is_safe() {
        let source = "x\u{2192}yz";
        let arrow_idx = source.find('\u{2192}').expect("arrow should exist");
        let range = range_to_lsp_range(arrow_idx + 1, arrow_idx + 2, source);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.end.line, 0);
        assert!(range.end.character >= range.start.character);
    }
}
