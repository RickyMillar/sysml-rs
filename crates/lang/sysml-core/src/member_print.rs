//! Declaration printers for the constructs the requirements workbench
//! AUTHORS (workbench design §7.3) — the server-side counterpart of the
//! buffer-writeback flow: the service composes an insertion edit from these
//! shapes; the client splices into the buffer (editor owns save).
//!
//! Deliberately NOT a general model-to-text serializer: one precisely-named
//! function per authored construct, nothing else (burden of proof is on
//! inclusion). Client-side templates are forbidden — string-shaping in the
//! FE is the fragility this module retires (the `create_scratch` token
//! rewrite precedent).
//!
//! All printers emit tab-indented text relative to `indent` (the parent
//! body's member indentation) and end WITHOUT a trailing newline; the
//! insertion composer owns surrounding whitespace.

/// `requirement <'REQ-042'> name { doc /* … */ }` — the R3 creation
/// skeleton. `short_name` is the spec's requirement ID (§7.21.2: reqId ≡
/// declaredShortName); `doc_body` is the statement text.
pub fn print_requirement_skeleton(
    short_name: Option<&str>,
    name: &str,
    doc_body: Option<&str>,
    indent: &str,
) -> String {
    let sn = short_name
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("<'{}'> ", s.trim()))
        .unwrap_or_default();
    match doc_body.map(str::trim).filter(|b| !b.is_empty()) {
        Some(body) => format!(
            "{indent}requirement {sn}{name} {{\n{indent}\t{}\n{indent}}}",
            print_doc_comment(body)
        ),
        None => format!("{indent}requirement {sn}{name};"),
    }
}

/// `doc /* body */` — a documentation comment member.
pub fn print_doc_comment(body: &str) -> String {
    format!("doc /* {} */", body.trim())
}

/// `@StatusInfo { status = StatusKind::tbd; }` — the spec's content-maturity
/// metadata (ModelingMetadata). The caller validates `status` against the
/// closed StatusKind vocabulary; this printer only shapes text.
pub fn print_status_info(status: &str) -> String {
    format!("@StatusInfo {{ status = StatusKind::{status}; }}")
}

/// `<keyword> <name> : <Type>;` — a typed parameter membership of a
/// requirement (§7.7). `keyword` is the closed set the caller validates:
/// `subject` / `actor` / `stakeholder` / `frame concern` (all
/// `ParameterMembership`s in the spec, same textual shape). `name` is the
/// local parameter name, `type_ref` the referenced definition.
pub fn print_typed_role(keyword: &str, name: &str, type_ref: &str) -> String {
    format!("{keyword} {name} : {type_ref};")
}

/// `require constraint <name> { <expr> }` / `assume constraint { <expr> }` — a
/// requirement constraint membership (§7.7). `is_assume` picks the keyword;
/// `name` is optional (the corpus names them, e.g. `minGap`); `expr` is the
/// boolean expression, spliced verbatim into the guarded braces. The caller
/// validates `expr` (single line, brace/`;`-free); this printer shapes text.
pub fn print_requirement_constraint(is_assume: bool, name: Option<&str>, expr: &str) -> String {
    let kw = if is_assume { "assume" } else { "require" };
    let named = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| format!("{n} "))
        .unwrap_or_default();
    format!("{kw} constraint {named}{{ {} }}", expr.trim())
}

/// `attribute <name> = <value>;` (or `attribute <name>;` when valueless) — an
/// attribute-usage declaration (§7.7). The caller validates `name` as an
/// identifier and `value` as a single-line expression; this printer shapes
/// text only.
pub fn print_attribute_declaration(name: &str, value: Option<&str>) -> String {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("attribute {name} = {v};"),
        None => format!("attribute {name};"),
    }
}

/// `@Rationale { text = "…"; }` — design-rationale metadata (ModelingMetadata;
/// read back via `metadata_string_attr(_, "text")`). `text` is embedded as a
/// string literal — this printer escapes `\` and `"` so arbitrary prose is
/// safe; the caller rejects control characters.
pub fn print_rationale(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("@Rationale {{ text = \"{escaped}\"; }}")
}

/// `satisfy <reqRef>;` — a satisfy statement written into the SATISFYING
/// feature's body (design §7.6: with no `by` clause the elaborator reads
/// the statement's owner as the satisfyingFeature, so the short form IS
/// "insert into the subject's body").
pub fn print_satisfy_statement(requirement_ref: &str) -> String {
    format!("satisfy {requirement_ref};")
}

/// `verify <reqRef>;` — a requirement-verification member inside an
/// EXISTING `objective` body (the only spec-legal home,
/// `validateRequirementVerificationMembershipOwningType`).
pub fn print_verify_statement(requirement_ref: &str) -> String {
    format!("verify {requirement_ref};")
}

/// `objective { verify <reqRef>; }` — the full objective block for a case
/// that has none yet (design §7.6). Named for the whole emitted shape, the
/// `print_requirement_skeleton` precedent.
pub fn print_objective_with_verify(requirement_ref: &str, indent: &str) -> String {
    format!("{indent}objective {{\n{indent}\tverify {requirement_ref};\n{indent}}}")
}

/// `#derivation connection { end #original ::> A; end #derive ::> B; }` —
/// the Requirement Derivation domain-library connection (design §7.6; the
/// core grammar has NO derive keyword). The load-bearing
/// `RequirementDerivation` import is the insertion composer's job, not this
/// printer's.
pub fn print_derivation_connection(
    original_ref: &str,
    derived_ref: &str,
    indent: &str,
) -> String {
    format!(
        "{indent}#derivation connection {{\n\
         {indent}\tend #original ::> {original_ref};\n\
         {indent}\tend #derive ::> {derived_ref};\n\
         {indent}}}"
    )
}

/// `dependency from <refining> to <refined> { @Refinement; }` — a refinement
/// (design §7.6). Refine is a plain KerML Dependency (client `from` →
/// supplier `to`) carrying a `ModelingMetadata::Refinement` annotation; the
/// anonymous form is spec-legal. The load-bearing `ModelingMetadata` import
/// is the insertion composer's job, not this printer's. (Trace is the same
/// shape minus the `@Refinement` member — a `print_trace_dependency` sibling
/// when trace earns a read surface, not built speculatively.)
pub fn print_refine_dependency(refining_ref: &str, refined_ref: &str, indent: &str) -> String {
    format!(
        "{indent}dependency from {refining_ref} to {refined_ref} {{\n\
         {indent}\t@Refinement;\n\
         {indent}}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_with_id_and_doc() {
        let s = print_requirement_skeleton(
            Some("REQ-042"),
            "tripTime",
            Some("The breaker shall trip within 40 ms."),
            "\t",
        );
        assert_eq!(
            s,
            "\trequirement <'REQ-042'> tripTime {\n\t\tdoc /* The breaker shall trip within 40 ms. */\n\t}"
        );
    }

    #[test]
    fn skeleton_minimal_is_a_semicolon_member() {
        assert_eq!(
            print_requirement_skeleton(None, "bare", None, "\t\t"),
            "\t\trequirement bare;"
        );
    }

    #[test]
    fn status_info_shape() {
        assert_eq!(
            print_status_info("tbd"),
            "@StatusInfo { status = StatusKind::tbd; }"
        );
    }

    #[test]
    fn typed_role_shape() {
        assert_eq!(
            print_typed_role("subject", "breaker", "Breaker"),
            "subject breaker : Breaker;"
        );
        assert_eq!(
            print_typed_role("actor", "installer", "ProtectionSpec::Installer"),
            "actor installer : ProtectionSpec::Installer;"
        );
    }

    #[test]
    fn requirement_constraint_shape() {
        assert_eq!(
            print_requirement_constraint(false, Some("minGap"), "gap >= 4.0"),
            "require constraint minGap { gap >= 4.0 }"
        );
        assert_eq!(
            print_requirement_constraint(true, None, "ambientTemp <= 40"),
            "assume constraint { ambientTemp <= 40 }"
        );
    }

    #[test]
    fn attribute_declaration_shape() {
        assert_eq!(
            print_attribute_declaration("maxTripTime", Some("40 [ms]")),
            "attribute maxTripTime = 40 [ms];"
        );
        assert_eq!(
            print_attribute_declaration("bare", None),
            "attribute bare;"
        );
    }

    #[test]
    fn rationale_shape_and_escaping() {
        assert_eq!(
            print_rationale("Threshold from the 2025 trade study."),
            "@Rationale { text = \"Threshold from the 2025 trade study.\"; }"
        );
        // Embedded quotes/backslashes are escaped into a valid literal.
        assert_eq!(
            print_rationale(r#"per "spec" \ ref"#),
            "@Rationale { text = \"per \\\"spec\\\" \\\\ ref\"; }"
        );
    }

    #[test]
    fn satisfy_and_verify_statements() {
        assert_eq!(print_satisfy_statement("tripTime"), "satisfy tripTime;");
        assert_eq!(
            print_verify_statement("ProtectionSpec::tripTime"),
            "verify ProtectionSpec::tripTime;"
        );
    }

    #[test]
    fn objective_block_shape() {
        assert_eq!(
            print_objective_with_verify("tripTime", "\t\t"),
            "\t\tobjective {\n\t\t\tverify tripTime;\n\t\t}"
        );
    }

    #[test]
    fn derivation_connection_shape() {
        assert_eq!(
            print_derivation_connection("ProtectionSpec::threshold", "coilSensitivity", "\t"),
            "\t#derivation connection {\n\t\tend #original ::> ProtectionSpec::threshold;\n\t\tend #derive ::> coilSensitivity;\n\t}"
        );
    }

    #[test]
    fn refine_dependency_shape() {
        assert_eq!(
            print_refine_dependency("coilSensitivity", "ProtectionSpec::threshold", "\t"),
            "\tdependency from coilSensitivity to ProtectionSpec::threshold {\n\t\t@Refinement;\n\t}"
        );
    }
}
