#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use sysml_codegen::{extract_all_keyword_strings, parse_xtext_enums, parse_xtext_operators};

/// Paths to xtext specification files relative to the references directory.
const SYSML_XTEXT_PATH: &str =
    "SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext";
const KERML_XTEXT_PATH: &str =
    "SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext";
const KERML_EXPRESSIONS_PATH: &str = "SysML-v2-Pilot-Implementation/org.omg.kerml.expressions.xtext/src/org/omg/kerml/expressions/xtext/KerMLExpressions.xtext";

/// Extra keywords that are used in our grammar but are not currently in xtext.
const EXTRA_KEYWORDS: &[&str] = &["readonly", "bool"];

fn main() -> Result<(), Box<dyn Error>> {
    let refs_dir = find_references_dir().ok_or("Could not find references/sysmlv2 directory")?;

    let sysml_xtext = fs::read_to_string(refs_dir.join(SYSML_XTEXT_PATH))?;
    let kerml_xtext = fs::read_to_string(refs_dir.join(KERML_XTEXT_PATH))?;
    let kerml_expr = fs::read_to_string(refs_dir.join(KERML_EXPRESSIONS_PATH))?;

    // Keywords
    let mut keyword_set = BTreeSet::new();
    keyword_set.extend(extract_all_keyword_strings(&sysml_xtext));
    keyword_set.extend(extract_all_keyword_strings(&kerml_xtext));
    keyword_set.extend(extract_all_keyword_strings(&kerml_expr));
    for extra in EXTRA_KEYWORDS {
        keyword_set.insert((*extra).to_owned());
    }
    let keywords: Vec<String> = keyword_set.into_iter().collect();

    // Operators (includes precedence + category)
    let operators = parse_xtext_operators(&kerml_expr);

    // Enums
    let enums = parse_xtext_enums(&sysml_xtext);
    let enums_by_type = build_enum_map(&enums);

    // Write generated JS files
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = manifest_dir.join("generated");
    fs::create_dir_all(&out_dir)?;

    write_keywords(&out_dir.join("keywords.js"), &keywords)?;
    write_operators(&out_dir.join("operators.js"), &operators)?;
    write_enums(&out_dir.join("enums.js"), &enums_by_type)?;

    println!(
        "Generated Tree-sitter token tables in {}",
        out_dir.display()
    );
    Ok(())
}

fn write_keywords(path: &Path, keywords: &[String]) -> Result<(), Box<dyn Error>> {
    let mut out = String::new();
    out.push_str("// Generated from SysML/KerML xtext files. Do not edit by hand.\n");
    out.push_str("module.exports = [\n");
    for kw in keywords {
        out.push_str(&format!("  \"{}\",\n", kw));
    }
    out.push_str("];\n");
    fs::write(path, out)?;
    Ok(())
}

fn write_operators(
    path: &Path,
    operators: &[sysml_codegen::OperatorInfo],
) -> Result<(), Box<dyn Error>> {
    let mut out = String::new();
    out.push_str("// Generated from KerMLExpressions.xtext. Do not edit by hand.\n");
    out.push_str("module.exports = [\n");
    for op in operators {
        out.push_str("  {\n");
        out.push_str(&format!("    name: \"{}\",\n", op.name));
        out.push_str(&format!("    category: \"{}\",\n", op.category));
        out.push_str(&format!("    precedence: {},\n", op.precedence));
        out.push_str("    symbols: [");
        for (idx, sym) in op.symbols.iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{}\"", sym));
        }
        out.push_str("],\n");
        out.push_str("  },\n");
    }
    out.push_str("];\n");
    fs::write(path, out)?;
    Ok(())
}

fn write_enums(path: &Path, enums: &BTreeMap<String, Vec<String>>) -> Result<(), Box<dyn Error>> {
    let mut out = String::new();
    out.push_str("// Generated from SysML.xtext enums. Do not edit by hand.\n");
    out.push_str("module.exports = {\n");
    for (enum_name, values) in enums {
        out.push_str(&format!("  \"{}\": [", enum_name));
        for (idx, val) in values.iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{}\"", val));
        }
        out.push_str("],\n");
    }
    out.push_str("};\n");
    fs::write(path, out)?;
    Ok(())
}

fn build_enum_map(enums: &[sysml_codegen::XtextEnumInfo]) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for en in enums {
        let name = extract_type_name(&en.returns_type).unwrap_or_else(|| en.name.clone());
        let entry = map.entry(name).or_default();
        for (_, value) in &en.values {
            if is_valid_keyword(value) {
                entry.insert(value.clone());
            }
        }
    }

    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

fn extract_type_name(return_type: &str) -> Option<String> {
    if let Some(pos) = return_type.rfind("::") {
        let name = &return_type[pos + 2..];
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}

fn is_valid_keyword(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
}

/// Find the sysmlv2 references directory by searching upward from the crate directory.
fn find_references_dir() -> Option<PathBuf> {
    if let Ok(refs_dir) = env::var("SYSML_REFS_DIR") {
        let path = PathBuf::from(refs_dir);
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(refs_dir) = env::var("SYSMLV2_REFS_DIR") {
        let path = PathBuf::from(refs_dir);
        if path.exists() {
            return Some(path);
        }
    }

    let mut current = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);

    for _ in 0..5 {
        let refs_path = current.join("references").join("sysmlv2");
        if refs_path.exists() && refs_path.is_dir() {
            return Some(refs_path);
        }

        if !current.pop() {
            break;
        }
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    if let Some(parent) = manifest_dir.parent() {
        if let Some(grandparent) = parent.parent() {
            let refs_path = grandparent.join("references").join("sysmlv2");
            if refs_path.exists() && refs_path.is_dir() {
                return Some(refs_path);
            }
        }
    }

    None
}
