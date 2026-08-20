//! Shared test fixtures for the cases test modules.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::cases::*;
use crate::expressions::ExprIR;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};

    /// Helper to build a simple RequirementCheck without sub-requirements.
    pub(crate) fn simple_req(
        id: &str,
        assumptions: Vec<ExprIR>,
        constraints: Vec<ExprIR>,
    ) -> RequirementCheck {
        RequirementCheck {
            id: id.into(),
            source_element_id: None,
            text: None,
            assumptions,
            constraint_element_ids: vec![None; constraints.len()],
            constraints,
            compile_errors: Vec::new(),
            subrequirements: Vec::new(),
            bindings: Vec::new(),
            binding_specs: Vec::new(),
        }
    }

    /// Helper to build a VerificationCaseIR with no setup actions.
    pub(crate) fn simple_case(id: &str, requirements: Vec<RequirementCheck>) -> VerificationCaseIR {
        VerificationCaseIR {
            id: id.into(),
            name: "Test".into(),
            subject: None,
            setup_actions: Vec::new(),
            requirements,
            sub_cases: Vec::new(),
            verdict_expression: None,
            bindings: Vec::new(),
        }
    }

    /// Helper to build a ModelGraph with a verification case and owned requirements.
    pub(crate) fn build_verification_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Add the verification case element
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("SpeedCheck")
            .with_prop("subject", Value::String("vehicle".into()));
        graph.add_element(vc);

        // Add a requirement owned by the verification case
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("speed-limit")
            .with_owner(vc_id.clone())
            .with_prop("text", Value::String("Speed must be under limit".into()))
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_element(req);

        graph
    }

    /// Build a graph mirroring:
    ///
    /// ```sysml
    /// constraint def BrewTempConstraint { in temp : Real; temp >= 90 and temp <= 96 }
    /// requirement def BrewTempReq { require constraint : BrewTempConstraint; }
    /// verification def BrewTempTest { objective { verify BrewTempReq; } }
    /// ```
    ///
    /// Uses unresolved_referencedFeature strings (pre-elaboration path) so the
    /// resolver fallback is exercised.
    pub(crate) fn build_verify_chain_graph() -> (ModelGraph, sysml_core::ElementId) {
        use sysml_core::ElementId;
        use sysml_parser_trait::relationship_builder::create_reference_subsetting;

        let mut graph = ModelGraph::new();

        // constraint def BrewTempConstraint { in temp; temp >= 90 and temp <= 96 }
        let constraint_def_id = ElementId::new_v4();
        let constraint_def =
            Element::new(constraint_def_id.clone(), ElementKind::ConstraintDefinition)
                .with_name("BrewTempConstraint")
                .with_prop(
                    "constraint",
                    Value::String("temp >= 90 and temp <= 96".into()),
                );
        graph.add_element(constraint_def);

        // requirement def BrewTempReq
        let req_id = ElementId::new_v4();
        let req = Element::new(req_id.clone(), ElementKind::RequirementDefinition)
            .with_name("BrewTempReq");
        graph.add_element(req);

        // `require constraint : BrewTempConstraint;` — child ConstraintUsage
        // of BrewTempReq with a ReferenceSubsetting pointing at BrewTempConstraint.
        let require_constraint_id = ElementId::new_v4();
        let require_constraint =
            Element::new(require_constraint_id.clone(), ElementKind::ConstraintUsage)
                .with_owner(req_id.clone());
        graph.add_element(require_constraint);
        create_reference_subsetting(
            &mut graph,
            require_constraint_id,
            "BrewTempConstraint".into(),
            None,
        );

        // verification def BrewTempTest
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("BrewTempTest");
        graph.add_element(vc);

        // objective { verify BrewTempReq; } — unnamed RequirementUsage child
        // of BrewTempTest with an unnamed ConstraintUsage child whose
        // ReferenceSubsetting targets BrewTempReq.
        let objective_id = ElementId::new_v4();
        let objective = Element::new(objective_id.clone(), ElementKind::RequirementUsage)
            .with_owner(vc_id.clone());
        graph.add_element(objective);

        let verify_usage_id = ElementId::new_v4();
        let verify_usage = Element::new(verify_usage_id.clone(), ElementKind::ConstraintUsage)
            .with_owner(objective_id);
        graph.add_element(verify_usage);
        create_reference_subsetting(&mut graph, verify_usage_id, "BrewTempReq".into(), None);

        (graph, vc_id)
    }
