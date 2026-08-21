//! Derived spec-reference index generation.
//!
//! Projects two kinds of grep-friendly artifacts from the normative sources
//! pinned in `references/sysmlv2/spec-drop.toml`:
//!
//! 1. **Clause-anchored spec plaintext** — the OMG spec HTML (12 MB / 8.5 MB)
//!    stripped to text, with every `view-title` heading emitted as a
//!    `## <clause-number> <title>` line so clause lookups are a plain grep.
//! 2. **Xtext rule → line-range index** — one row per grammar rule so
//!    `KerML.xtext:NNN`-style citations can be resolved (and re-anchored
//!    after an upstream renumbering) by rule name.
//!
//! Both artifacts carry the source file's SHA-256 in their header. They are
//! never hand-edited: a regen-diff test in `sysml-spec-tests` re-runs this
//! generator and fails on any difference, and a checksum test ties the
//! recorded source hashes back to `spec-drop.toml`.
//!
//! The extraction is deliberately dependency-free (no HTML parser crate):
//! the spec HTML is a machine-produced MMS/Angular export with regular
//! structure, and the derived text only has to be a faithful *view* for
//! grepping — the HTML stays the normative source.

use sha2::{Digest, Sha256};

pub mod language_pack;

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Spec HTML → clause-anchored plaintext
// ---------------------------------------------------------------------------

/// Tags treated as block-level: a newline is emitted in their place so
/// paragraphs, list items, and table cells land on their own lines.
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "br"
            | "li"
            | "ul"
            | "ol"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "figure"
            | "figcaption"
            | "blockquote"
            | "pre"
            | "hr"
    )
}

/// Decode the handful of HTML entities that actually occur in the spec
/// export. Unknown entities are kept verbatim (they are then greppable,
/// which beats silently eating them).
fn decode_entity(entity: &str) -> Option<String> {
    Some(match entity {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" => "'".to_owned(),
        "nbsp" => " ".to_owned(),
        _ => {
            let code = if let Some(hex) = entity.strip_prefix("#x").or(entity.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok()?
            } else {
                return None;
            };
            char::from_u32(code)?.to_string()
        }
    })
}

/// Strip HTML to text. `<style>`/`<script>` contents and comments are
/// dropped; block tags become newlines, inline tags become spaces;
/// `<h1 class="view-title ...">` blocks (the MMS clause headings) are
/// prefixed with `## ` so clause numbers anchor the output.
pub fn extract_spec_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 4);
    let mut i = 0;
    // Inside an `<h1 class="view-title ...">` block: keep the clause number
    // and title on ONE output line (source newlines/nested tags become
    // spaces) so `## 8.3.6.4 <title>` anchors are grep-able as a unit.
    let mut in_heading = false;

    while i < bytes.len() {
        let rest = html.get(i..).unwrap_or_default();
        if rest.starts_with('<') {
            // Comment?
            if rest.starts_with("<!--") {
                i = rest.find("-->").map_or(bytes.len(), |p| i + p + 3);
                continue;
            }
            // Scan the tag, honouring quoted attribute values.
            let tag_start = i + 1;
            let mut j = tag_start;
            let mut quote: Option<u8> = None;
            while let Some(&b) = bytes.get(j) {
                match (quote, b) {
                    (Some(q), c) if c == q => quote = None,
                    (None, b'"') | (None, b'\'') => quote = Some(b),
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            let tag_body = html.get(tag_start..j.min(bytes.len())).unwrap_or_default();
            i = (j + 1).min(bytes.len());

            let closing = tag_body.starts_with('/');
            let name: String = tag_body
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();

            // Drop style/script contents entirely.
            if !closing && (name == "style" || name == "script") {
                let close = format!("</{name}");
                let content = html.get(i..).unwrap_or_default();
                if let Some(p) = content.to_ascii_lowercase().find(&close) {
                    // Skip past the closing tag's '>'.
                    let after = i + p;
                    i = html
                        .get(after..)
                        .unwrap_or_default()
                        .find('>')
                        .map_or(bytes.len(), |g| after + g + 1);
                } else {
                    i = bytes.len();
                }
                continue;
            }

            // Clause heading anchor: MMS view titles are h1.view-title.
            if name == "h1" && !closing && tag_body.contains("view-title") {
                out.push_str("\n\n## ");
                in_heading = true;
                continue;
            }
            if name == "h1" && closing && in_heading {
                in_heading = false;
                out.push('\n');
                continue;
            }

            if is_block_tag(&name) && !in_heading {
                out.push('\n');
            } else {
                out.push(' ');
            }
            continue;
        }

        if rest.starts_with('&') {
            // Entity: &name; or &#NNN; (bounded scan).
            if let Some(semi) = rest
                .get(1..)
                .unwrap_or_default()
                .find(';')
                .filter(|&p| p <= 10)
            {
                if let Some(decoded) = rest.get(1..1 + semi).and_then(decode_entity) {
                    out.push_str(&decoded);
                    i += semi + 2;
                    continue;
                }
            }
            out.push('&');
            i += 1;
            continue;
        }

        // Plain text: push the full character (multi-byte safe). In heading
        // mode, source newlines flatten to spaces to keep one heading line.
        let Some(ch) = rest.chars().next() else { break };
        if in_heading && ch == '\n' {
            out.push(' ');
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }

    // Normalise: collapse intra-line whitespace, drop blank-line runs.
    let mut lines: Vec<String> = Vec::new();
    for raw in out.lines() {
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(collapsed);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Assemble the full derived spec-text artifact: provenance header + text.
/// `source_rel` is the path relative to `references/sysmlv2/` (matching the
/// `[[source]].path` key in spec-drop.toml).
pub fn spec_text_artifact(source_rel: &str, html: &str) -> String {
    format!(
        "# source: {source_rel}\n\
         # sha256: {sha}\n\
         # generated-by: tools/spec-index (cargo run -p spec-index)\n\
         # DO NOT EDIT — regenerated and diffed by the derived_indexes gate in\n\
         # sysml-spec-tests; the HTML above is the normative source, this file\n\
         # is a grep-friendly view of it.\n\
         \n\
         {text}",
        sha = sha256_hex(html.as_bytes()),
        text = extract_spec_text(html),
    )
}

// ---------------------------------------------------------------------------
// Xtext grammar → rule/line index
// ---------------------------------------------------------------------------

/// One grammar rule occupying `line_start..=line_end` (1-based, inclusive).
pub struct XtextRule {
    pub name: String,
    pub line_start: usize,
    pub line_end: usize,
}

/// A rule header is a non-indented line ending with `:` — optionally
/// prefixed `fragment`/`terminal`/`enum` — that is not the `grammar` or an
/// `import` declaration. Every rule in the three pinned grammars follows
/// this shape (verified across KerML/SysML/KerMLExpressions .xtext); the
/// regen-diff gate would surface any future formatting drift.
fn rule_header_name(line: &str) -> Option<String> {
    if !line.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if !line.trim_end().ends_with(':') {
        return None;
    }
    let mut rest = line;
    for prefix in ["terminal", "fragment", "enum"] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            if stripped.starts_with(' ') {
                rest = stripped.trim_start();
            }
        }
    }
    if rest.starts_with("grammar ") || rest.starts_with("import ") {
        return None;
    }
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Extract every rule with its line range. A rule ends on the last
/// non-empty line before the next rule header (or end of file).
pub fn extract_xtext_rules(src: &str) -> Vec<XtextRule> {
    let lines: Vec<&str> = src.lines().collect();
    let mut headers: Vec<(usize, String)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(name) = rule_header_name(line) {
            headers.push((idx + 1, name));
        }
    }

    let mut rules = Vec::with_capacity(headers.len());
    for (h, (start, name)) in headers.iter().enumerate() {
        let hard_end = headers
            .get(h + 1)
            .map_or(lines.len(), |(next_start, _)| next_start - 1);
        // Trim trailing blank lines off the range.
        let mut end = hard_end;
        while end > *start
            && lines
                .get(end - 1)
                .is_some_and(|line| line.trim().is_empty())
        {
            end -= 1;
        }
        rules.push(XtextRule {
            name: name.clone(),
            line_start: *start,
            line_end: end,
        });
    }
    rules
}

/// Assemble the xtext-rules TOML artifact from `(section, source_rel,
/// grammar_source)` triples. Sections group rules per grammar file; rows are
/// `RuleName = { start = N, end = M }`.
pub fn xtext_rules_artifact(grammars: &[(&str, &str, &str)]) -> String {
    let mut out = String::new();
    out.push_str(
        "# Xtext rule -> line-range index (1-based, inclusive).\n\
         # generated-by: tools/spec-index (cargo run -p spec-index)\n\
         # DO NOT EDIT — regenerated and diffed by the derived_indexes gate in\n\
         # sysml-spec-tests. Cite grammar rules by name via this index; raw\n\
         # line-number citations rot when upstream renumbers.\n",
    );
    for (section, source_rel, src) in grammars {
        out.push_str(&format!(
            "\n# source: {source_rel}\n# sha256: {}\n[{section}]\n",
            sha256_hex(src.as_bytes())
        ));
        for rule in extract_xtext_rules(src) {
            out.push_str(&format!(
                "{} = {{ start = {}, end = {} }}\n",
                rule.name, rule.line_start, rule.line_end
            ));
        }
    }
    out
}
