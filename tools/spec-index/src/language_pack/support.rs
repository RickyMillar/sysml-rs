//! Machine-derived implementation-support axes from evidence records.
//! Support values are never hand-written prose: they are
//! computed from `evidence-record.schema.json` data at the manifest commit.
//!
//! Hard rule: a stale commit, a missing report, a generic-parser-fallback
//! acceptance, or hand-edited prose can never yield `validated`. Evidence whose
//! `commit` != the manifest commit is downgraded to `unknown`.

use serde::{Deserialize, Serialize};

/// The 8 support axes. Serializes to the card `support`
/// object (`support-status.schema.json#/$defs/supportAxes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupportAxes {
    pub parse: String,
    pub lower: String,
    pub resolve: String,
    pub elaborate: String,
    pub validate: String,
    pub execute: String,
    pub format: String,
    pub lsp: String,
}

impl SupportAxes {
    /// All axes `unknown` — the honest default before any gate evidence exists.
    pub fn all_unknown() -> Self {
        let u = || "unknown".to_owned();
        SupportAxes {
            parse: u(),
            lower: u(),
            resolve: u(),
            elaborate: u(),
            validate: u(),
            execute: u(),
            format: u(),
            lsp: u(),
        }
    }

    fn set(&mut self, axis: &str, value: String) {
        match axis {
            "parse" => self.parse = value,
            "lower" => self.lower = value,
            "resolve" => self.resolve = value,
            "elaborate" => self.elaborate = value,
            "validate" => self.validate = value,
            "execute" => self.execute = value,
            "format" => self.format = value,
            "lsp" => self.lsp = value,
            _ => {}
        }
    }
}

/// One evidence datum (matches `evidence-record.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub card_id: String,
    pub axis: String,
    pub commit: String,
    pub gate: String,
    pub case_id: String,
    pub result: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_gap_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justifies: Option<String>,
}

/// Derive the 8 axes for a card from its evidence records at `manifest_commit`.
///
/// Per-axis policy:
/// - only records whose `commit == manifest_commit` count; others are ignored
///   (forcing the stale axis to `unknown`);
/// - all matching records `pass` and at least one present -> `validated`;
/// - at least one `pass` but a mix -> `partial`;
/// - a `fail`/`generic-fallback` carrying a reviewed `known_gap_ref` that
///   `justifies` `unsupported` -> `unsupported`;
/// - otherwise -> `unknown` (including no evidence at all, e.g. `execute`).
pub fn derive_axes(
    card_id: &str,
    manifest_commit: &str,
    evidence: &[EvidenceRecord],
) -> SupportAxes {
    let mut axes = SupportAxes::all_unknown();
    for axis in [
        "parse", "lower", "resolve", "elaborate", "validate", "execute", "format", "lsp",
    ] {
        let matching: Vec<&EvidenceRecord> = evidence
            .iter()
            .filter(|e| e.card_id == card_id && e.axis == axis && e.commit == manifest_commit)
            .collect();
        if matching.is_empty() {
            continue; // stays unknown
        }
        let passes = matching.iter().filter(|e| e.result == "pass").count();
        let value = if passes == matching.len() {
            "validated"
        } else if passes > 0 {
            "partial"
        } else if matching.iter().any(|e| {
            e.known_gap_ref.is_some() && e.justifies.as_deref() == Some("unsupported")
        }) {
            "unsupported"
        } else {
            "unknown"
        };
        axes.set(axis, value.to_owned());
    }
    axes
}
