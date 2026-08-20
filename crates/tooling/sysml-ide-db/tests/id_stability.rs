//! Reparse/re-elaboration id-stability pins (ADR-009 follow-up to the
//! content-true fingerprint, workspace-scope plan §W6b).
//!
//! With `ModelGraph::fingerprint` content-true (ids included), salsa
//! backdating only works if identical input produces identical ids.
//! Parse- and elaboration-tier mints must therefore be CanonicalKey-
//! stable; a single fresh-UUID mint on these paths makes every
//! recompute compare unequal and defeats downstream memoization.
//!
//! The fixture deliberately exercises membership minting (nested
//! members), implicit generalization (a part def with no explicit
//! supertype), and feature typing — the paths that route through
//! `MembershipBuilder` / `create_owning_membership*`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use sysml_ide_db::{RootDatabase, SourceFile};

const MODEL: &str = r#"
package Fixture {
    part def Vehicle {
        attribute mass : ScalarValues::Real = 1500.0;
    }
    part car : Vehicle {
        doc /* the family car */
    }
    requirement def R1 {
        doc /* must carry five people */
    }
}
"#;

/// Parsing the same file+content in two fresh databases yields equal
/// ParseResults — parse-tier ids and memberships are reparse-stable.
#[test]
fn reparse_is_id_stable() {
    let db1 = RootDatabase::default();
    let db2 = RootDatabase::default();
    let sf1 = SourceFile::new(&db1, "fixture.sysml".to_string(), MODEL.to_string());
    let sf2 = SourceFile::new(&db2, "fixture.sysml".to_string(), MODEL.to_string());
    let r1 = sysml_ide_db::parse_file(&db1, sf1);
    let r2 = sysml_ide_db::parse_file(&db2, sf2);
    assert_eq!(
        r1, r2,
        "re-parse of identical input must produce identical ids/content \
         (a fresh-UUID mint on the parse path defeats salsa backdating)"
    );
}

/// Elaborating the same file+content in two fresh databases yields equal
/// ElaboratedModels — elaboration-tier mints (implicit generalization
/// memberships included) are re-run-stable.
#[test]
fn re_elaboration_is_id_stable() {
    let db1 = RootDatabase::default();
    let db2 = RootDatabase::default();
    let sf1 = SourceFile::new(&db1, "fixture.sysml".to_string(), MODEL.to_string());
    let sf2 = SourceFile::new(&db2, "fixture.sysml".to_string(), MODEL.to_string());
    let e1 = sysml_ide_db::elaborate_file_best(&db1, sf1, None, None);
    let e2 = sysml_ide_db::elaborate_file_best(&db2, sf2, None, None);
    assert_eq!(
        e1, e2,
        "re-elaboration of identical input must produce identical ids/content \
         (a fresh-UUID mint on the elaboration path defeats salsa backdating)"
    );
}
