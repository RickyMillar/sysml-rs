//! Document-idiom gate — the spec's intended requirements-document shape
//! (§7.21 + the pilot HSUV/Annex-A.8 idioms), end-to-end through
//! parse → elaborate → rows/detail.
//!
//! Purpose-built fixture (mirrors the requirements-document idiom):
//! a numbered tree of requirement USAGES as the document, one shared
//! obligation REFERENCED (never re-instantiated — `require shared;`),
//! and a def used as a genuine template with two bound instantiations.
//!
//! Gates:
//! 1. the reference form `require <existingUsage>;` parses to a
//!    RequirementConstraintMembership — NOT a phantom requirement row and
//!    NOT a silently-dropped anonymous feature (the pre-fix failure mode),
//! 2. the referenced requirement stays ONE row,
//! 3. the reference surfaces in the referencing requirement's contract as
//!    a resolved reference,
//! 4. template instantiations are rows with inherited contract +
//!    binding-value display (`:>> gap = 8.0` shows name `gap`),
//! 5. the def's `instantiated_by` lists both instantiations.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

const FIXTURE: &str = r#"package DocIdiom {
	part def Breaker;

	requirement <'4'> spec {
		subject breaker : Breaker;
		requirement <'4.1'> tripTime {
			doc /* Shall trip within 40 ms. */
		}
		requirement <'4.3'> insulation {
			requirement <'4.3.1'> busCreepage : CreepageRule {
				attribute :>> gap = 8.0;
			}
			requirement <'4.3.2'> relayCreepage : CreepageRule {
				attribute :>> gap = 4.0;
			}
		}
		require emc;
	}

	requirement <'9'> emc {
		doc /* Device-wide EMC compliance — one obligation, referenced. */
	}

	requirement def CreepageRule {
		attribute gap;
		require constraint minGap { gap >= 4.0 }
	}
}
"#;

fn open_fixture() -> SysmlService {
    let dir = std::env::temp_dir().join(format!("sysml-doc-idiom-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("DocIdiom.sysml"), FIXTURE).expect("write fixture");
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(PathBuf::from(dir)))
        .expect("open document-idiom fixture workspace");
    service
}

fn rows(service: &SysmlService) -> Vec<serde_json::Value> {
    execute_command(
        service,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("requirement_rows must succeed")["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

fn detail(service: &SysmlService, id: &str) -> serde_json::Value {
    execute_command(
        service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": id }),
    )
    .expect("requirement_detail must succeed")
}

#[test]
fn document_idiom_reference_form_and_template_instantiation() {
    let service = open_fixture();
    let all = rows(&service);
    let names: Vec<&str> = all.iter().filter_map(|r| r["name"].as_str()).collect();

    // 1+2. The document tree, one row per content node — the reference
    //      creates NO phantom and `emc` appears exactly once.
    assert_eq!(
        names,
        vec![
            "spec",
            "tripTime",
            "insulation",
            "busCreepage",
            "relayCreepage",
            "emc",
            "CreepageRule"
        ],
        "document order, one row per content node"
    );
    let ids: std::collections::HashMap<&str, &str> = all
        .iter()
        .filter_map(|r| Some((r["name"].as_str()?, r["id"].as_str()?)))
        .collect();

    // Outline depths mirror the clause nesting.
    let depth = |name: &str| {
        all.iter()
            .find(|r| r["name"] == name)
            .and_then(|r| r["outline_depth"].as_u64())
    };
    assert_eq!(depth("spec"), Some(0));
    assert_eq!(depth("tripTime"), Some(1));
    assert_eq!(depth("busCreepage"), Some(2));
    assert_eq!(depth("emc"), Some(0));

    // 3. The reference form surfaces in spec's contract, resolved to emc.
    let spec_detail = detail(&service, ids["spec"]);
    let required = spec_detail["required_constraints"]
        .as_array()
        .expect("required_constraints");
    let reference = required
        .iter()
        .find(|c| c["referenced_definition"]["name"] == "emc")
        .unwrap_or_else(|| {
            panic!("`require emc;` must surface as a resolved reference: {required:?}")
        });
    assert!(
        reference["text"].is_null(),
        "the reference owns no inline body"
    );

    // 4. Template instantiations: inherited contract with provenance,
    //    binding value displayed under the redefined feature's name.
    let bus = detail(&service, ids["busCreepage"]);
    let inherited = bus["inherited_required_constraints"]
        .as_array()
        .expect("inherited bucket");
    assert_eq!(inherited.len(), 1);
    assert_eq!(inherited[0]["text"].as_str(), Some("gap >= 4.0"));
    assert_eq!(
        inherited[0]["inherited_from"]["name"].as_str(),
        Some("CreepageRule")
    );
    let attrs = bus["referenced_attributes"].as_array().expect("attrs");
    assert_eq!(
        attrs
            .iter()
            .map(|a| (a["name"].as_str(), a["value"].as_str()))
            .collect::<Vec<_>>(),
        vec![(Some("gap"), Some("8"))],
        "the :>> binding displays under the redefined feature's name"
    );

    // 5. The def lists both content instantiations.
    let def_detail = detail(&service, ids["CreepageRule"]);
    assert_eq!(
        def_detail["instantiated_by"]
            .as_array()
            .expect("instantiated_by")
            .iter()
            .map(|r| r["name"].as_str().unwrap_or("<unnamed>"))
            .collect::<Vec<_>>(),
        vec!["busCreepage", "relayCreepage"]
    );
}
