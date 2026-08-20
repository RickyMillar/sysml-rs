//! ONE home for a service-computed text replacement.
//!
//! Rename, formatting, diagram edits, code actions, and the requirements
//! workbench field edits all return this shape (workbench design §7.2 —
//! the four byte-identical per-module structs were collapsed here; never
//! reintroduce a per-command copy). Coordinates are line/character in
//! UTF-16 code units, 0-indexed — the LSP convention, so the LSP shim is a
//! 1:1 translation and Monaco (UTF-16-native) applies them directly.

/// One text replacement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextEdit {
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub new_text: String,
    /// Staleness guard: the exact text currently occupying the edited range
    /// in the source the edit was computed against. A client applying this
    /// edit to a BUFFER (not an LSP-tracked document) MUST verify the buffer
    /// slice equals this before splicing and fail loudly on mismatch —
    /// never a silent mis-splice (workbench design §7.2). `None` when the
    /// producer has no meaningful prior text for the range (e.g. pure
    /// whitespace formatting runs, insertions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_old_text: Option<String>,
}
