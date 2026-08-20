//! B1/B1b gate — Derive/Refine/Trace service answers, including the
//! spec-idiomatic RequirementDerivation user-defined-keyword form.

use std::path::{Path, PathBuf};

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

const EXPLICIT_FIXTURE: &str = r#"package B1Explicit {
    private import DerivationConnections::*;
    private import ModelingMetadata::*;

    requirement vehicleMassRequirement;
    requirement chassisMassRequirement;
    part amplifier;

    connection derivation : Derivation {
        end ref originalEnd references vehicleMassRequirement;
        end ref derivedEnd references chassisMassRequirement;
    }
    dependency refineDep from chassisMassRequirement to vehicleMassRequirement {
        @Refinement;
    }
    dependency traceDep from amplifier to vehicleMassRequirement;
}
"#;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture_workspace(label: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sysml-b1-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("create B1 fixture directory");
    std::fs::write(dir.join("B1Fixture.sysml"), source).expect("write B1 fixture");
    dir
}

fn matrix_names(
    service: &SysmlService,
    source_kind: &str,
    rel_kind: &str,
    target_kind: &str,
) -> Vec<(String, String)> {
    let value = execute_command(
        service,
        "sysml.trace_matrix",
        json!({
            "uri": "__workspace__",
            "source_kind": source_kind,
            "rel_kind": rel_kind,
            "target_kind": target_kind,
        }),
    )
    .unwrap_or_else(|e| panic!("trace_matrix({rel_kind}) failed: {e:?}"));
    value
        .as_array()
        .expect("trace_matrix array")
        .iter()
        .map(|row| {
            (
                row["source_name"].as_str().unwrap_or("").to_owned(),
                row["target_name"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn open_fixture(label: &str, source: &str) -> SysmlService {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_workspace(label, source)))
        .expect("open B1 fixture workspace");
    service
}

#[test]
fn b1_explicit_derive_refine_trace_answers() {
    let service = open_fixture("explicit", EXPLICIT_FIXTURE);
    assert_eq!(
        matrix_names(&service, "RequirementUsage", "derive", "RequirementUsage"),
        vec![(
            "chassisMassRequirement".into(),
            "vehicleMassRequirement".into()
        )]
    );
    assert_eq!(
        matrix_names(&service, "RequirementUsage", "refine", "RequirementUsage"),
        vec![(
            "chassisMassRequirement".into(),
            "vehicleMassRequirement".into()
        )]
    );
    assert_eq!(
        matrix_names(&service, "PartUsage", "trace", "RequirementUsage"),
        vec![("amplifier".into(), "vehicleMassRequirement".into())]
    );
}

#[test]
fn b1b_keyword_derivation_matches_explicit_trace_answer() {
    let explicit = open_fixture("explicit-equivalence", EXPLICIT_FIXTURE);
    let explicit_rows = matrix_names(&explicit, "RequirementUsage", "derive", "RequirementUsage");

    let keyword_source = std::fs::read_to_string(
        workspace_root()
            .join("crates/lang/sysml-parser-incremental/tests/fixtures")
            .join("g24-keyword-derivation.sysml"),
    )
    .expect("read keyword derivation fixture");
    let keyword = open_fixture("keyword-equivalence", &keyword_source);
    let keyword_rows = matrix_names(&keyword, "RequirementUsage", "derive", "RequirementUsage");

    assert_eq!(
        keyword_rows,
        vec![(
            "chassisMassRequirement".into(),
            "vehicleMassRequirement".into()
        )],
        "keyword form must elaborate derived → original"
    );
    assert_eq!(
        keyword_rows, explicit_rows,
        "keyword and explicit forms must produce identical trace_matrix answers"
    );
}
