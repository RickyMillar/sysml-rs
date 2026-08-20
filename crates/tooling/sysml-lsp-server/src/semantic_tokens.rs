//! Semantic token building and CST walking for syntax highlighting.
//!
//! Contains the SemanticTokensBuilder that handles multiline token splitting
//! and delta encoding, plus model-graph and CST-based token emission.

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

use std::sync::atomic::Ordering;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{SemanticToken, Position, SemanticTokensParams, SemanticTokensResult, SemanticTokens, SemanticTokensRangeParams, SemanticTokensRangeResult, SemanticTokensDeltaParams, SemanticTokensFullDeltaResult, SemanticTokensDelta, SemanticTokensEdit};

use sysml_ide_db::Cancelled;

use crate::types::{MOD_ABSTRACT, MOD_DEFINITION, MOD_DERIVED, MOD_READONLY, MOD_UNRESOLVED};
use crate::utils::{position_to_offset, LineIndex};
use crate::SysmlLanguageServer;

/// Translate a `sysml_ide_db` modifier bitfield into the LSP wire legend.
///
/// The crate-internal `token_modifiers` bits and the LSP
/// `SEMANTIC_TOKEN_MODIFIERS` legend do **not** share bit positions: ABSTRACT is
/// bit 1 internally but legend index 4, and UNRESOLVED is bit 4 internally but
/// legend index 6. Forwarding the raw bitfield mislabels tokens (an abstract
/// token would read as `declaration`, index 1). This is the single, explicit
/// translation point — every LSP semantic-token path routes modifiers through
/// it. (READONLY/DERIVED/DEFINITION happen to share bit positions, but we map
/// them explicitly too so the legend stays the sole source of truth.)
fn remap_modifiers(ide: u32) -> u32 {
    use sysml_ide_db::tokens::token_modifiers as m;
    let mut out = 0u32;
    if ide & m::DEFINITION != 0 {
        out |= MOD_DEFINITION;
    }
    if ide & m::ABSTRACT != 0 {
        out |= MOD_ABSTRACT;
    }
    if ide & m::READONLY != 0 {
        out |= MOD_READONLY;
    }
    if ide & m::DERIVED != 0 {
        out |= MOD_DERIVED;
    }
    if ide & m::UNRESOLVED != 0 {
        out |= MOD_UNRESOLVED;
    }
    out
}

/// Helper for building semantic tokens.
pub(crate) struct SemanticTokensBuilder<'a> {
    source: &'a str,
    tokens: Vec<(usize, usize, u32, u32)>, // (start, end, type, modifiers)
}

impl<'a> SemanticTokensBuilder<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        SemanticTokensBuilder {
            source,
            tokens: Vec::new(),
        }
    }

    pub(crate) fn add_token(&mut self, start: usize, end: usize, token_type: u32, modifiers: u32) {
        self.tokens.push((start, end, token_type, modifiers));
    }

    pub(crate) fn build(mut self) -> Vec<SemanticToken> {
        // Sort by start position
        self.tokens.sort_by_key(|(start, _, _, _)| *start);

        // Expand multiline tokens into per-line tokens (LSP spec requires single-line tokens)
        // Build the line index once: each token does two offset->Position
        // conversions, so a per-call scan would be O(tokens * file_len).
        let line_index = LineIndex::new(self.source);
        let mut expanded = Vec::new();
        for (start, end, token_type, modifiers) in &self.tokens {
            let start_pos = line_index.position(*start);
            let end_pos = line_index.position(*end);

            if start_pos.line == end_pos.line {
                expanded.push((
                    start_pos,
                    end_pos.character - start_pos.character,
                    *token_type,
                    *modifiers,
                ));
            } else {
                // Split into per-line tokens
                let lines: Vec<&str> = self.source.lines().collect();

                // First line: from start_pos.character to end of line
                if let Some(line) = lines.get(start_pos.line as usize) {
                    let line_len = line.chars().map(|c| c.len_utf16() as u32).sum::<u32>();
                    let length = line_len.saturating_sub(start_pos.character);
                    if length > 0 {
                        expanded.push((start_pos, length, *token_type, *modifiers));
                    }
                }

                // Middle lines: full line
                for line_num in (start_pos.line + 1)..end_pos.line {
                    if let Some(line) = lines.get(line_num as usize) {
                        let trimmed = line.trim_start();
                        if !trimmed.is_empty() {
                            let leading = (line.len() - trimmed.len()) as u32;
                            let length = trimmed.chars().map(|c| c.len_utf16() as u32).sum::<u32>();
                            let pos = Position {
                                line: line_num,
                                character: leading,
                            };
                            expanded.push((pos, length, *token_type, *modifiers));
                        }
                    }
                }

                // Last line: from start to end_pos.character
                if end_pos.character > 0 {
                    let pos = Position {
                        line: end_pos.line,
                        character: 0,
                    };
                    expanded.push((pos, end_pos.character, *token_type, *modifiers));
                }
            }
        }

        // Sort expanded tokens by (line, character, length-ascending) so the
        // narrower of two tokens starting at the same position comes first.
        // The overlap-collapse pass below then keeps the narrower one — the
        // model token for a qualified-name reference (e.g. `Pkg::Member`) is
        // wider than the CST identifier tokens that should paint each segment;
        // we want to keep the segments, not the whole-name token.
        expanded.sort_by(|a, b| {
            a.0.line
                .cmp(&b.0.line)
                .then(a.0.character.cmp(&b.0.character))
                .then(a.1.cmp(&b.1))
        });

        // Overlap collapse. The LSP spec allows overlapping tokens only when
        // the client advertises `overlappingTokenSupport`; our simulation-app
        // client (editors/simulation-app/src/features/editor/lspClient.ts)
        // does not, so dedupe is server responsibility. Walk in order, drop
        // any token whose `(line, character)` falls inside the previously
        // kept token's `(line, character..character+length)` range.
        let mut deduped: Vec<(Position, u32, u32, u32)> = Vec::with_capacity(expanded.len());
        for (pos, length, token_type, modifiers) in expanded {
            if let Some(last) = deduped.last() {
                let same_line = last.0.line == pos.line;
                let inside = same_line
                    && pos.character >= last.0.character
                    && pos.character < last.0.character.saturating_add(last.1);
                if inside {
                    continue;
                }
            }
            deduped.push((pos, length, token_type, modifiers));
        }

        let mut result = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_start = 0u32;

        for (pos, length, token_type, modifiers) in deduped {
            let delta_line = pos.line - prev_line;
            let delta_start = if delta_line == 0 {
                pos.character - prev_start
            } else {
                pos.character
            };

            result.push(SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset: modifiers,
            });

            prev_line = pos.line;
            prev_start = pos.character;
        }

        result
    }
}

// The LSP crate no longer carries its own CST token walker. All three token
// paths (full / delta / range) now consume the single salsa `file_semantic_tokens`
// query in `sysml-ide-db`, which owns the model + reference + CST walkers. This
// removed one of the three duplicate CST walkers the codebase carried (principle
// 4/5). The remaining divergent walker lives only in `snapshot_tests.rs` (a
// test-render helper), tracked in sysml-ide-db/src/tokens.rs's module docs.

pub(crate) async fn semantic_tokens_full(
    server: &SysmlLanguageServer,
    params: SemanticTokensParams,
) -> Result<Option<SemanticTokensResult>> {
    let uri = params.text_document.uri.to_string();
    let guard = server
        .pending_requests
        .begin(uri.clone(), "semantic_tokens");

    let (sf, analysis) = match server.salsa_file_context(&uri).await {
        Some(ctx) => ctx,
        None => {
            server.pending_requests.end(&guard);
            return Ok(None);
        }
    };
    let project_id = server.file_project_id(&uri).await;

    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let content = analysis.file_text(sf).to_owned();

        // Get raw tokens from salsa (memoized model + reference + CST extraction)
        let raw_tokens = analysis.semantic_tokens(sf, project_id);

        // Convert raw tokens to LSP format via SemanticTokensBuilder
        let mut tokens_builder = SemanticTokensBuilder::new(&content);
        for raw in raw_tokens.tokens() {
            let token_type = token_category_to_lsp(raw.category);
            tokens_builder.add_token(raw.start, raw.end, token_type, remap_modifiers(raw.modifiers));
        }

        tokens_builder.build()
    }));

    let tokens = match result {
        Ok(tokens) => tokens,
        Err(_cancelled) => {
            server.pending_requests.end(&guard);
            return Ok(None);
        }
    };

    if !server.pending_requests.is_current(&guard) {
        server.pending_requests.end(&guard);
        return Ok(None);
    }
    server.pending_requests.end(&guard);

    let result_id = server
        .semantic_token_counter
        .fetch_add(1, Ordering::Relaxed)
        .to_string();
    server
        .last_semantic_tokens
        .insert(uri.clone(), (result_id.clone(), tokens.clone()));
    Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
        result_id: Some(result_id),
        data: tokens,
    })))
}

pub(crate) async fn semantic_tokens_range(
    server: &SysmlLanguageServer,
    params: SemanticTokensRangeParams,
) -> Result<Option<SemanticTokensRangeResult>> {
    let uri = params.text_document.uri.to_string();
    let range = params.range;

    // The range path routes through the SAME salsa `file_semantic_tokens` query
    // as the full/delta paths (model + resolution-backed reference + CST tokens)
    // and simply filters to the requested byte range. This collapses what was a
    // third, divergent manual walker over the parse graph (principle 4/5): it
    // now picks up the resolution-backed reference tokens the walker lacked and
    // shares the one modifier remap, so a range request can never disagree with
    // a full request on colour or modifiers.
    let (sf, analysis) = match server.salsa_file_context(&uri).await {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let project_id = server.file_project_id(&uri).await;

    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let content = analysis.file_text(sf).to_owned();
        let range_start = position_to_offset(&range.start, &content);
        let range_end = position_to_offset(&range.end, &content);
        let raw_tokens = analysis.semantic_tokens(sf, project_id);

        let mut tokens_builder = SemanticTokensBuilder::new(&content);
        for raw in raw_tokens.tokens() {
            // Keep tokens overlapping the requested range.
            if raw.end <= range_start || raw.start >= range_end {
                continue;
            }
            tokens_builder.add_token(
                raw.start,
                raw.end,
                token_category_to_lsp(raw.category),
                remap_modifiers(raw.modifiers),
            );
        }
        tokens_builder.build()
    }));

    let tokens = match result {
        Ok(tokens) => tokens,
        Err(_cancelled) => return Ok(None),
    };
    Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
        result_id: None,
        data: tokens,
    })))
}

pub(crate) async fn semantic_tokens_full_delta(
    server: &SysmlLanguageServer,
    params: SemanticTokensDeltaParams,
) -> Result<Option<SemanticTokensFullDeltaResult>> {
    let uri = params.text_document.uri.to_string();
    let previous_result_id = params.previous_result_id;

    // Look up cached tokens for this URI
    let old_tokens = server.last_semantic_tokens.get(&uri).and_then(|entry| {
        let (cached_id, tokens) = entry.value();
        if cached_id == &previous_result_id {
            Some(tokens.clone())
        } else {
            None
        }
    });

    // Compute new tokens via salsa (memoized raw tokens + LSP conversion)
    let (sf, analysis) = match server.salsa_file_context(&uri).await {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let project_id = server.file_project_id(&uri).await;

    let new_tokens = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let content = analysis.file_text(sf).to_owned();
        let raw_tokens = analysis.semantic_tokens(sf, project_id);
        let mut tokens_builder = SemanticTokensBuilder::new(&content);
        for raw in raw_tokens.tokens() {
            tokens_builder.add_token(
                raw.start,
                raw.end,
                token_category_to_lsp(raw.category),
                remap_modifiers(raw.modifiers),
            );
        }
        tokens_builder.build()
    })) {
        Ok(tokens) => tokens,
        Err(_cancelled) => return Ok(None),
    };

    let result_id = server
        .semantic_token_counter
        .fetch_add(1, Ordering::Relaxed)
        .to_string();
    server
        .last_semantic_tokens
        .insert(uri.clone(), (result_id.clone(), new_tokens.clone()));

    match old_tokens {
        Some(old) => {
            let edits = compute_semantic_token_edits(&old, &new_tokens);
            Ok(Some(SemanticTokensFullDeltaResult::TokensDelta(
                SemanticTokensDelta {
                    result_id: Some(result_id),
                    edits,
                },
            )))
        }
        None => {
            // Cache miss: fall back to full tokens
            Ok(Some(SemanticTokensFullDeltaResult::Tokens(
                SemanticTokens {
                    result_id: Some(result_id),
                    data: new_tokens,
                },
            )))
        }
    }
}

pub(crate) fn token_category_to_lsp(category: sysml_ide_db::TokenCategory) -> u32 {
    use sysml_ide_db::TokenCategory;
    match category {
        TokenCategory::Namespace => 0,
        TokenCategory::Type => 1,
        TokenCategory::Class => 2,
        TokenCategory::Struct => 3,
        TokenCategory::Property => 4,
        TokenCategory::Variable => 5,
        TokenCategory::Parameter => 6,
        TokenCategory::Function => 7,
        TokenCategory::Keyword => 8,
        TokenCategory::Comment => 9,
        TokenCategory::String => 10,
        TokenCategory::Number => 11,
        TokenCategory::Operator => 12,
        TokenCategory::Interface => 13,
        TokenCategory::Enum => 14,
    }
}

/// Compute a minimal set of edits to transform `old` semantic tokens into `new`.
///
/// LSP `textDocument/semanticTokens/full/delta` expects edits against the
/// flattened `u32` representation of the token array. Each `SemanticToken` is
/// (5 values per token). `start` and `delete_count` reference positions in
/// that flat array, while `data` provides replacement `SemanticToken` objects
/// (which tower-lsp serializes to the flat format).
pub(crate) fn compute_semantic_token_edits(
    old: &[SemanticToken],
    new: &[SemanticToken],
) -> Vec<SemanticTokensEdit> {
    if old == new {
        return Vec::new();
    }

    // Find common prefix length (in tokens)
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    // Find common suffix length (in tokens)
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old.len() - prefix)
        .min(new.len() - prefix);

    let old_mid_len = old.len() - prefix - suffix;
    let new_mid = &new[prefix..new.len() - suffix];

    if old_mid_len == 0 && new_mid.is_empty() {
        return Vec::new();
    }

    // start and delete_count are in the flat u32 array (5 values per token)
    vec![SemanticTokensEdit {
        start: (prefix * 5) as u32,
        delete_count: (old_mid_len * 5) as u32,
        data: Some(new_mid.to_vec()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper — decode the delta-encoded `SemanticToken` stream back into
    /// absolute `(line, char_start, length, token_type)` tuples for assertions.
    fn decode(toks: &[SemanticToken]) -> Vec<(u32, u32, u32, u32)> {
        let mut out = Vec::new();
        let mut line = 0u32;
        let mut start = 0u32;
        for t in toks {
            line += t.delta_line;
            if t.delta_line != 0 {
                start = 0;
            }
            start += t.delta_start;
            out.push((line, start, t.length, t.token_type));
        }
        out
    }

    #[test]
    fn overlap_collapse_drops_token_nested_inside_earlier_one() {
        // Source has two tokens on line 0: a 10-char-wide one starting at 0,
        // and a 3-char-wide one at column 4 (fully inside the first).
        // Expectation: keep the first, drop the nested.
        let mut b = SemanticTokensBuilder::new("                    "); // dummy
        b.add_token(0, 10, 1, 0);
        b.add_token(4, 7, 2, 0);
        let out = decode(&b.build());
        assert_eq!(out, vec![(0, 0, 10, 1)]);
    }

    #[test]
    fn overlap_collapse_prefers_narrower_at_same_start() {
        // Two tokens at the same start: 10 wide and 3 wide.
        // The sort tiebreaker is length-ascending, so the narrower wins;
        // the wider one starts inside it (same start position) and is dropped.
        let mut b = SemanticTokensBuilder::new("                    ");
        b.add_token(0, 10, 1, 0);
        b.add_token(0, 3, 2, 0);
        let out = decode(&b.build());
        assert_eq!(out, vec![(0, 0, 3, 2)]);
    }

    #[test]
    fn overlap_collapse_keeps_adjacent_non_overlapping_tokens() {
        // Two tokens that touch but do not overlap: 0..5 and 5..10.
        let mut b = SemanticTokensBuilder::new("                    ");
        b.add_token(0, 5, 1, 0);
        b.add_token(5, 10, 2, 0);
        let out = decode(&b.build());
        assert_eq!(out, vec![(0, 0, 5, 1), (0, 5, 5, 2)]);
    }

    #[test]
    fn overlap_collapse_keeps_tokens_on_different_lines() {
        // Same character position but on different lines — must both survive.
        let src = "aaa\nbbb\n";
        let mut b = SemanticTokensBuilder::new(src);
        b.add_token(0, 3, 1, 0); // line 0 chars 0..3
        b.add_token(4, 7, 2, 0); // line 1 chars 0..3 (after newline)
        let out = decode(&b.build());
        assert_eq!(out, vec![(0, 0, 3, 1), (1, 0, 3, 2)]);
    }
}
