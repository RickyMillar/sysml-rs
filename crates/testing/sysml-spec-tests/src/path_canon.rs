//! Canonicalize absolute checkout paths in test baselines to stable,
//! checkout-independent tokens — the ONE home for every baseline-writing
//! test (`cross_transport_identity_baseline`, `service_command_baseline`)
//! so committed snapshots and JSON archives never embed a developer's
//! absolute path. Portability requirement: a fresh clone (or the
//! history-rewrite rehearsal at a different path) must reproduce these
//! baselines byte-for-byte.
//!
//! This is NOT a redaction hack layered on top of the model: the service
//! *correctly* echoes the absolute URI a file was loaded under (LSP needs
//! absolute `file://` URIs to talk to an editor). Portable baselines
//! require the *test* — which chose to load fixtures under absolute paths —
//! to project those absolute locators onto canonical repo-relative ones
//! before it commits them. That projection happens exactly once, over the
//! whole response bundle, at the serialization seam — never per-field, and
//! never by widening a per-shape redaction regex.
//!
//! Scope note: this canonicalizes the literal path *strings* a response
//! carries. It does NOT make `content_digest` / ElementIds checkout-stable
//! — those embed the absolute path through `CanonicalKey::root(file_path)`
//! and are fixed at the source (ADR-009 root-key seam), not here.

use serde_json::Value;

/// A single absolute-prefix → stable-token substitution.
pub struct PathReplacement {
    /// Absolute filesystem prefix as it appears in response strings
    /// (e.g. the workspace root or the sibling `the-book` root).
    pub from: String,
    /// Checkout-independent token to substitute (e.g. `"<WS>"`).
    pub to: &'static str,
}

impl PathReplacement {
    pub fn new(from: impl Into<String>, to: &'static str) -> Self {
        PathReplacement {
            from: from.into(),
            to,
        }
    }
}

/// Recursively replace every occurrence of each replacement's `from` prefix
/// with its `to` token across every string in `value` — object values,
/// object keys, and array elements alike. Substring replacement is
/// deliberate: it catches the path wherever it surfaces — a bare `path` /
/// `root` field, a `file://` scheme URI, or a path embedded mid-sentence in
/// a human-readable `status_message`.
///
/// Replacements are applied in the given order per string, so callers must
/// list the most specific (longest) prefixes first: a parent directory that
/// is a prefix of another replacement's `from` must come last, or it would
/// partially rewrite the longer paths and erase the distinction.
pub fn canonicalize_paths(value: &mut Value, replacements: &[PathReplacement]) {
    match value {
        Value::String(s) => {
            apply_to_string(s, replacements);
        }
        Value::Array(items) => {
            for item in items {
                canonicalize_paths(item, replacements);
            }
        }
        Value::Object(map) => {
            let key_needs_rewrite = map
                .keys()
                .any(|k| replacements.iter().any(|r| k.contains(&r.from)));
            if key_needs_rewrite {
                // A key carries a path — rebuild the map so the key is
                // canonicalized too (serde_json keys are immutable in place).
                let taken = std::mem::take(map);
                let mut rebuilt = serde_json::Map::with_capacity(taken.len());
                for (mut key, mut val) in taken {
                    apply_to_string(&mut key, replacements);
                    canonicalize_paths(&mut val, replacements);
                    rebuilt.insert(key, val);
                }
                *map = rebuilt;
            } else {
                for val in map.values_mut() {
                    canonicalize_paths(val, replacements);
                }
            }
        }
        _ => {}
    }
}

fn apply_to_string(s: &mut String, replacements: &[PathReplacement]) {
    for r in replacements {
        if s.contains(&r.from) {
            *s = s.replace(&r.from, r.to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reps() -> Vec<PathReplacement> {
        vec![
            PathReplacement::new("/abs/root/sysml-rs", "<WS>"),
            PathReplacement::new("/abs/root/the-book", "<BOOK>"),
            PathReplacement::new("/abs/root", "<REPO>"),
        ]
    }

    #[test]
    fn rewrites_file_uri_bare_path_and_embedded_message() {
        let mut v = json!({
            "uri": "file:///abs/root/sysml-rs/examples/x.sysml",
            "root": "/abs/root/the-book/examples/coffee-machine",
            "status_message": "Created PartDefinition in /abs/root/sysml-rs/x.sysml",
        });
        canonicalize_paths(&mut v, &reps());
        assert_eq!(v["uri"], "file://<WS>/examples/x.sysml");
        assert_eq!(v["root"], "<BOOK>/examples/coffee-machine");
        assert_eq!(
            v["status_message"],
            "Created PartDefinition in <WS>/x.sysml"
        );
    }

    #[test]
    fn specific_prefix_wins_over_parent() {
        // The parent `/abs/root` must not partially rewrite the longer
        // workspace path — order (specific first) guarantees it.
        let mut v = json!({ "p": "/abs/root/sysml-rs/a" });
        canonicalize_paths(&mut v, &reps());
        assert_eq!(v["p"], "<WS>/a");
    }

    #[test]
    fn standalone_parent_still_caught() {
        let mut v = json!({ "p": "/abs/root/misc/y" });
        canonicalize_paths(&mut v, &reps());
        assert_eq!(v["p"], "<REPO>/misc/y");
    }

    #[test]
    fn rewrites_object_keys() {
        let mut v = json!({ "/abs/root/sysml-rs/a.sysml": 1 });
        canonicalize_paths(&mut v, &reps());
        assert_eq!(v, json!({ "<WS>/a.sysml": 1 }));
    }

    #[test]
    fn leaves_unrelated_strings_untouched() {
        let mut v = json!({ "name": "PumpODE", "n": 42 });
        canonicalize_paths(&mut v, &reps());
        assert_eq!(v, json!({ "name": "PumpODE", "n": 42 }));
    }
}
