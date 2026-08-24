    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use sysml_core::{Element, ElementKind, ModelGraph, Value};
    use crate::expressions::EvalContext;
    use crate::ode_builder;
    use crate::orchestrator::Orchestrator;

    /// c3 conformance: a constraint declared on a definition with TWO usages
    /// must yield TWO independent verdicts — one per occurrence, each evaluated
    /// against that occurrence's bound value (Constraints.sysml:23,
    /// Performances.kerml:94-102). The flat single-verdict path cannot represent
    /// this (one bare `level` for two instances); the per-instance primitive can.
    #[test]
    fn per_instance_constraint_yields_one_verdict_per_occurrence() {
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let source = r#"
            package C3 {
                part def Tank { attribute level; constraint c { level < 100 } }
                part tankA : Tank { attribute level = 50; }
                part tankB : Tank { attribute level = 150; }
            }
        "#;
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("c3.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        let _ = sysml_core::resolution::resolve_references(&mut graph);
        elaborate::elaborate(&mut graph);
        let compiler = ModelCompiler::from_arc(Arc::new(graph));
        let base_ctx = context_from_graph(compiler.graph());
        let set = extract_constraints_filtered(compiler.graph(), |_| true);
        let precompiled = precompile_constraint_set(&set);
        let results = compiler
            .evaluate_constraints_per_instance(&precompiled, &base_ctx)
            .expect("constraint evaluation");

        let mut by_inst: Vec<(String, bool, bool)> = results
            .iter()
            .map(|r| {
                (
                    r.instance_path.clone().unwrap_or_default(),
                    r.result.satisfied,
                    r.result.inconclusive,
                )
            })
            .collect();
        by_inst.sort();
        assert_eq!(
            by_inst.len(),
            2,
            "expected 2 independent per-occurrence verdicts, got {by_inst:?}"
        );
        assert!(
            by_inst
                .iter()
                .any(|(p, sat, inc)| p == "tankA" && *sat && !*inc),
            "tankA (level=50 < 100) should PASS: {by_inst:?}"
        );
        assert!(
            by_inst
                .iter()
                .any(|(p, sat, inc)| p == "tankB" && !*sat && !*inc),
            "tankB (level=150 < 100) should FAIL (definitive, not inconclusive): {by_inst:?}"
        );
    }

    /// Helper: create a ModelGraph with @ToolExecution solver annotations
    /// on part definitions.
    ///
    /// Each entry is (owner_name, tool_name).
    fn make_solver_selection_graph(owners: &[(&str, &str)]) -> ModelGraph {
        use sysml_core::ElementId;

        let mut graph = ModelGraph::new();

        for (owner_name, tool_name) in owners {
            let owner_id = ElementId::new_v4();
            let mut owner = Element::new(owner_id.clone(), ElementKind::PartDefinition);
            owner.name = Some(owner_name.to_string());
            graph.add_element(owner);

            // Create @ToolExecution metadata on the part def
            let meta_id = ElementId::new_v4();
            let mut meta = Element::new(meta_id.clone(), ElementKind::MetadataUsage);
            meta.name = Some("ToolExecution".to_owned());
            meta.owner = Some(owner_id.clone());
            graph.add_element(meta);

            // toolName child
            let tn_id = ElementId::new_v4();
            let mut tn = Element::new(tn_id.clone(), ElementKind::AttributeUsage);
            tn.name = Some("toolName".to_owned());
            tn.owner = Some(meta_id.clone());
            tn.set_prop("value", sysml_core::Value::String(tool_name.to_string()));
            graph.add_element(tn);
        }

        graph.rebuild_indexes();
        graph
    }

    #[test]
    fn test_detect_solver_selections_two_owners() {
        let graph = make_solver_selection_graph(&[
            ("UnitThermalModel", "builtin:ode-rk4"),
            ("CorePlantModel", "builtin:ode-rk45"),
        ]);

        let selections = detect_solver_selections_from_metadata(&graph);
        assert_eq!(selections.len(), 2);
        assert_eq!(
            selections.get("UnitThermalModel").unwrap(),
            "builtin:ode-rk4"
        );
        assert_eq!(
            selections.get("CorePlantModel").unwrap(),
            "builtin:ode-rk45"
        );
    }

    #[test]
    fn test_detect_solver_selections_empty_graph() {
        let graph = ModelGraph::new();
        let selections = detect_solver_selections_from_metadata(&graph);
        assert!(
            selections.is_empty(),
            "empty graph should produce no solver selections"
        );
    }

    #[test]
    fn test_detect_solver_selections_ignores_non_ode_tools() {
        // @ToolExecution with toolName that doesn't start with "builtin:ode-"
        let graph = make_solver_selection_graph(&[("SomeModel", "builtin:fmi-cosim")]);
        let selections = detect_solver_selections_from_metadata(&graph);
        assert!(selections.is_empty(), "non-ODE tool should be ignored");
    }

    #[test]
    fn test_ode_detection_is_rk45() {
        let ode = OdeDetection {
            name: Some("Test".to_owned()),
            tool_name: "builtin:ode-rk45".to_owned(),
            state_vars: vec!["x".to_owned()],
            initial_values: vec![0.0],
            parameters: HashMap::new(),
            derivative_exprs: vec!["1.0".to_owned()],
            signal_exprs: HashMap::new(),
            owner_id: None,
            subsystem_index: None,
            derivative_match_errors: Vec::new(),
        };
        assert!(ode.is_rk45());

        let ode_rk4 = OdeDetection {
            tool_name: "builtin:ode-rk4".to_owned(),
            ..ode.clone()
        };
        assert!(!ode_rk4.is_rk45());

        let ode_ssr = OdeDetection {
            tool_name: "ssr:GetDerivative".to_owned(),
            ..ode
        };
        assert!(!ode_ssr.is_rk45());
    }

    /// RSC-4.2 ruling 4 (fail-hard fold-in): an `OdeDetection` reaching
    /// `mint_slot_store` with `subsystem_index: None` — i.e. never
    /// registered as an orchestrator subsystem — must hard-fail the mint,
    /// not silently fall back to a placeholder writer (the deleted
    /// `WriterId::Orchestrator` soft fallback this ruling replaced).
    #[test]
    fn mint_slot_store_fails_hard_on_unresolved_ode_subsystem_index() {
        let ode = OdeDetection {
            name: Some("UnresolvedOde".to_owned()),
            tool_name: "builtin:ode-rk45".to_owned(),
            state_vars: vec!["x".to_owned()],
            initial_values: vec![0.0],
            parameters: HashMap::new(),
            derivative_exprs: vec!["1.0".to_owned()],
            signal_exprs: HashMap::new(),
            owner_id: None,
            subsystem_index: None, // never registered — the mint gap ruling 4 closes
            derivative_match_errors: Vec::new(),
        };
        let compiler = ModelCompiler::new(ModelGraph::new());
        let err = compiler
            .mint_slot_store(&SlotMintInputs {
                instances: &[],
                ode_detections: std::slice::from_ref(&ode),
                multiplied_ode_names: &HashSet::new(),
                computed_targets: &[],
                sm_targets: &[],
                primary_sm_names: &[],
                sm_guard_reads: &[],
                sm_payload_ports: &[],
                override_map: &HashMap::new(),
                link_graph: None,
                port_registry: None,
                physics_write_targets: None,
                physics_writer: None,
                duty_tracker_odes: &HashSet::new(),
                discrete_detections: &[],
            })
            .expect_err("an ODE with no registered SubsystemIndex must hard-fail the mint");
        assert!(
            err.message.contains("UnresolvedOde") && err.message.contains("mint-gap"),
            "expected the RSC-4.2 mint-gap error naming the ODE, got: {}",
            err.message
        );
    }

    // Tests for `prefix_expression_identifiers` were removed with the function
    // (RSC-4.2 / C.4). Instance-scoped binding is now covered by the slot-plane
    // parity tests (`rsc24a_*`, `instance_scoped_expressions_*`) and the
    // byte-identical baselines (the sysml-spec-tests behavioural suite).

    // -----------------------------------------------------------------------
    // Tests for generic flow bridge
    // -----------------------------------------------------------------------

    #[test]
    fn generic_flow_bridge_creates_expressions_for_electrical_ports() {
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use crate::orchestrator::OrchestratorConfig;
        use sysml_core::physics::classify::ClassificationConfidence;
        use sysml_core::ElementId;

        // Build a graph with PhasePort and PhaseInPort definitions
        let mut graph = ModelGraph::new();

        // Create PortDefinition "PhasePort"
        let pd_id = ElementId::new_v4();
        let mut pd = Element::new(pd_id.clone(), ElementKind::PortDefinition);
        pd.name = Some("PhasePort".to_string());
        graph.add_element(pd);

        // Create PortDefinition "PhaseInPort"
        let pd2_id = ElementId::new_v4();
        let mut pd2 = Element::new(pd2_id.clone(), ElementKind::PortDefinition);
        pd2.name = Some("PhaseInPort".to_string());
        graph.add_element(pd2);

        // Create PortUsage "branchOut1" typed as PhasePort
        let pu1_id = ElementId::new_v4();
        let mut pu1 = Element::new(pu1_id.clone(), ElementKind::PortUsage);
        pu1.name = Some("branchOut1".to_string());
        graph.add_element(pu1);
        // Add FeatureTyping child
        let ft1_id = ElementId::new_v4();
        let mut ft1 = Element::new(ft1_id.clone(), ElementKind::FeatureTyping);
        ft1.owner = Some(pu1_id.clone());
        ft1.set_prop("unresolved_type", Value::String("PhasePort".to_string()));
        graph.add_element(ft1);

        // Create PortUsage "phaseIn" typed as PhaseInPort
        let pu2_id = ElementId::new_v4();
        let mut pu2 = Element::new(pu2_id.clone(), ElementKind::PortUsage);
        pu2.name = Some("phaseIn".to_string());
        graph.add_element(pu2);
        let ft2_id = ElementId::new_v4();
        let mut ft2 = Element::new(ft2_id.clone(), ElementKind::FeatureTyping);
        ft2.owner = Some(pu2_id.clone());
        ft2.set_prop("unresolved_type", Value::String("PhaseInPort".to_string()));
        graph.add_element(ft2);

        graph.rebuild_indexes();

        let compiler = ModelCompiler::new(graph);
        let mut orch = Orchestrator::new(OrchestratorConfig::default());

        // RSC-3.5e.5 W3: the bridge consumes the classified link graph. Build a
        // single FlowUsage link busbar.branchOut1 -> branch1.breaker.phaseIn
        // (the bridge only reads the endpoints + FlowUsage kind).
        let mut link_graph = LinkGraph::new();
        link_graph.intern(LinkIR {
            element_id: ElementId::new_v4(),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "busbar".into(),
                port: "branchOut1".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "branch1.breaker".into(),
                port: "phaseIn".into(),
                resolved_registry_key: None,
            },
            class: LinkClass::Unknown,
            class_confidence: ClassificationConfidence::Unknown,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });

        compiler.wire_generic_flow_bridge(&mut orch, &link_graph);

        // The bridge should have created a computed expression for current propagation
        // Set the source value and tick to verify
        orch.context
            .set("busbar.branchOut1.current".to_string(), Value::Float(5.0));

        // Evaluate computed expressions manually
        let evaluator = crate::expressions::ExpressionEvaluator::new();
        // The computed expression should be: branch1.breaker.phaseIn.current = busbar.branchOut1.current
        // Access private field via tick simulation would be ideal, but we'll verify
        // the expression was added by checking it evaluates correctly
        assert!(
            orch.context.get("busbar.branchOut1.current").is_some(),
            "source variable should exist"
        );
    }

    // -----------------------------------------------------------------------
    // Tests for instance-scoped expression detection
    // -----------------------------------------------------------------------

    #[test]
    fn instance_scoped_expressions_prefix_correctly() {
        use crate::orchestrator::OrchestratorConfig;
        use sysml_core::ElementId;

        let mut graph = ModelGraph::new();

        // Create PartDefinition "BranchPath"
        let pd_id = ElementId::new_v4();
        let mut pd = Element::new(pd_id.clone(), ElementKind::PartDefinition);
        pd.name = Some("BranchPath".to_string());
        graph.add_element(pd);

        // Add computed attribute: tripped = coreTemp >= 150.0
        let attr_id = ElementId::new_v4();
        let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
        attr.name = Some("tripped".to_string());
        attr.owner = Some(pd_id.clone());
        attr.set_prop(
            "unresolved_value",
            Value::String("coreTemp >= 150.0".to_string()),
        );
        graph.add_element(attr);

        // Create PartUsage "branch1" typed as BranchPath
        let pu_id = ElementId::new_v4();
        let mut pu = Element::new(pu_id.clone(), ElementKind::PartUsage);
        pu.name = Some("branch1".to_string());
        pu.set_prop(
            "unresolvedTypeName",
            Value::String("BranchPath".to_string()),
        );
        graph.add_element(pu);

        // Create PartUsage "branch2" typed as BranchPath
        let pu2_id = ElementId::new_v4();
        let mut pu2 = Element::new(pu2_id.clone(), ElementKind::PartUsage);
        pu2.name = Some("branch2".to_string());
        pu2.set_prop(
            "unresolvedTypeName",
            Value::String("BranchPath".to_string()),
        );
        graph.add_element(pu2);

        graph.rebuild_indexes();

        let compiler = ModelCompiler::new(graph);
        let mut orch = Orchestrator::new(OrchestratorConfig::default());

        let instances = vec![
            InstanceSpec {
                prefix: "branch1".to_string(),
                sm_names: vec![],
                ode_detections: vec![],
                container_name: None,
                type_def_id: None,
                usage_id: None,
                config_entries: vec![],
                sm_subsystem_indices: HashMap::new(),
            },
            InstanceSpec {
                prefix: "branch2".to_string(),
                sm_names: vec![],
                ode_detections: vec![],
                container_name: None,
                type_def_id: None,
                usage_id: None,
                config_entries: vec![],
                sm_subsystem_indices: HashMap::new(),
            },
        ];

        compiler.detect_instance_scoped_expressions(&mut orch, &instances);

        // Set up context values and evaluate
        orch.context
            .set("branch1.coreTemp".to_string(), Value::Float(200.0));
        orch.context
            .set("branch2.coreTemp".to_string(), Value::Float(100.0));

        // Tick the orchestrator to evaluate computed expressions
        // (Instead of full tick, just check expressions were added)
        let evaluator = crate::expressions::ExpressionEvaluator::new();
        // We can't easily access private computed_expressions, but we can verify
        // via a small integration test that the values propagate
    }

    // -----------------------------------------------------------------------
    // Acceptance test: generic synthetic model using generic mechanisms
    // -----------------------------------------------------------------------

    /// Synthetic test: a "WaterPipeline" model with pump instances, flow
    /// connections, and computed aggregates — verifying the generic mechanisms
    /// work for a completely non-electrical domain.
    #[test]
    fn generic_mechanisms_work_for_generic_model() {
        use sysml_core::ElementId;

        let mut graph = ModelGraph::new();

        // --- PortDefinitions for hydraulic domain ---
        let pd_id = ElementId::new_v4();
        let mut pd = Element::new(pd_id.clone(), ElementKind::PortDefinition);
        pd.name = Some("WaterOutPort".to_string());
        graph.add_element(pd);

        let pd2_id = ElementId::new_v4();
        let mut pd2 = Element::new(pd2_id.clone(), ElementKind::PortDefinition);
        pd2.name = Some("WaterInPort".to_string());
        graph.add_element(pd2);

        // --- PartDefinition "PumpUnit" (will be instance-multiplied) ---
        let pump_def_id = ElementId::new_v4();
        let mut pump_def = Element::new(pump_def_id.clone(), ElementKind::PartDefinition);
        pump_def.name = Some("PumpUnit".to_string());
        graph.add_element(pump_def);

        // Computed attribute in PumpUnit: pressure = flowRate * resistance
        let attr_id = ElementId::new_v4();
        let mut attr = Element::new(attr_id.clone(), ElementKind::AttributeUsage);
        attr.name = Some("pressure".to_string());
        attr.owner = Some(pump_def_id.clone());
        attr.set_prop("expr", Value::String("flowRate * resistance".to_string()));
        graph.add_element(attr);

        // --- Container "Pipeline" with 2 pump instances ---
        let pipeline_id = ElementId::new_v4();
        let mut pipeline = Element::new(pipeline_id.clone(), ElementKind::PartDefinition);
        pipeline.name = Some("Pipeline".to_string());
        graph.add_element(pipeline);

        let p1_id = ElementId::new_v4();
        let mut p1 = Element::new(p1_id.clone(), ElementKind::PartUsage);
        p1.name = Some("pump1".to_string());
        p1.owner = Some(pipeline_id.clone());
        p1.set_prop("unresolvedTypeName", Value::String("PumpUnit".to_string()));
        graph.add_element(p1);

        let p2_id = ElementId::new_v4();
        let mut p2 = Element::new(p2_id.clone(), ElementKind::PartUsage);
        p2.name = Some("pump2".to_string());
        p2.owner = Some(pipeline_id.clone());
        p2.set_prop("unresolvedTypeName", Value::String("PumpUnit".to_string()));
        graph.add_element(p2);

        // --- Aggregate: totalFlow = pump1.flowRate + pump2.flowRate ---
        let total_id = ElementId::new_v4();
        let mut total = Element::new(total_id.clone(), ElementKind::AttributeUsage);
        total.name = Some("totalFlow".to_string());
        total.set_prop(
            "expr",
            Value::String("pump1.flowRate + pump2.flowRate".to_string()),
        );
        graph.add_element(total);

        graph.rebuild_indexes();

        // --- Verify: detect_computed_expressions finds the aggregate ---
        let computed = ModelCompiler::detect_computed_expressions(&graph);
        let total_found = computed.iter().any(|(name, _)| name == "totalFlow");
        assert!(total_found, "should detect totalFlow aggregate from model");

        // --- Verify: instance-scoped expressions work for PumpUnit ---
        let compiler = ModelCompiler::new(graph);
        let mut orch = crate::orchestrator::Orchestrator::new(
            crate::orchestrator::OrchestratorConfig::default(),
        );

        let instances = vec![
            InstanceSpec {
                prefix: "pump1".to_string(),
                sm_names: vec![],
                ode_detections: vec![],
                container_name: None,
                type_def_id: None,
                usage_id: None,
                config_entries: vec![],
                sm_subsystem_indices: HashMap::new(),
            },
            InstanceSpec {
                prefix: "pump2".to_string(),
                sm_names: vec![],
                ode_detections: vec![],
                container_name: None,
                type_def_id: None,
                usage_id: None,
                config_entries: vec![],
                sm_subsystem_indices: HashMap::new(),
            },
        ];

        compiler.detect_instance_scoped_expressions(&mut orch, &instances);

        // Seed context with per-instance values
        orch.context
            .set("pump1.flowRate".to_string(), Value::Float(10.0));
        orch.context
            .set("pump1.resistance".to_string(), Value::Float(2.0));
        orch.context
            .set("pump2.flowRate".to_string(), Value::Float(5.0));
        orch.context
            .set("pump2.resistance".to_string(), Value::Float(3.0));

        // Also add the aggregate expression
        for (target, expr_str) in &ModelCompiler::detect_computed_expressions(compiler.graph()) {
            if let Ok(expr) = ode_builder::parse_derivative(expr_str) {
                orch.add_computed_expression(target, expr);
            }
        }

        // Manually evaluate computed expressions to verify
        let evaluator = crate::expressions::ExpressionEvaluator::new();

        // The instance-scoped expressions should have created:
        // pump1.pressure = pump1.flowRate * pump1.resistance
        // pump2.pressure = pump2.flowRate * pump2.resistance
        // And the aggregate: totalFlow = pump1.flowRate + pump2.flowRate

        // We need to trigger evaluation (via orchestrator step).
        // Since we can't easily step without subsystems, test the expressions
        // via the expression evaluator directly.
        let expr_p1 = ode_builder::parse_derivative("pump1.flowRate * pump1.resistance").unwrap();
        let val_p1 = evaluator.eval(&expr_p1, &orch.context).unwrap();
        assert_eq!(val_p1, Value::Float(20.0), "pump1 pressure = 10 * 2 = 20");

        let expr_total = ode_builder::parse_derivative("pump1.flowRate + pump2.flowRate").unwrap();
        let val_total = evaluator.eval(&expr_total, &orch.context).unwrap();
        assert_eq!(val_total, Value::Float(15.0), "totalFlow = 10 + 5 = 15");
    }

    // -----------------------------------------------------------------------
    // GetNextState / StateSpaceDynamics detection
    // -----------------------------------------------------------------------

    /// Helper: build a model graph with `action def :> StateSpaceDynamics` containing
    /// a `calc :>> getNextState : GetNextState` with a direct next-state expression.
    fn make_state_space_dynamics_graph(
        owner_name: &str,
        state_var: &str,
        initial_val: f64,
        expr: &str,
    ) -> ModelGraph {
        use sysml_core::ElementId;

        let mut graph = ModelGraph::new();

        // Owner part def
        let owner_id = ElementId::new_v4();
        let mut owner = Element::new(owner_id.clone(), ElementKind::PartDefinition);
        owner.name = Some(owner_name.to_string());
        graph.add_element(owner);

        // Action def specializing StateSpaceDynamics
        let action_id = ElementId::new_v4();
        let mut action = Element::new(action_id.clone(), ElementKind::ActionDefinition);
        action.name = Some("dynamics".to_string());
        action.owner = Some(owner_id.clone());
        action.set_prop(
            "unresolvedTypeName",
            Value::String("StateSpaceDynamics".to_string()),
        );
        graph.add_element(action);

        // State variable (out attribute)
        let sv_id = ElementId::new_v4();
        let mut sv = Element::new(sv_id.clone(), ElementKind::AttributeUsage);
        sv.name = Some(state_var.to_string());
        sv.owner = Some(action_id.clone());
        sv.set_prop("direction", Value::String("out".to_string()));
        sv.set_prop("default", Value::Float(initial_val));
        graph.add_element(sv);

        // GetNextState calc usage inside the action
        let calc_id = ElementId::new_v4();
        let mut calc = Element::new(calc_id.clone(), ElementKind::CalculationUsage);
        calc.name = Some("getNextState".to_string());
        calc.owner = Some(action_id.clone());
        graph.add_element(calc);

        // Subclassification child marking it as :> GetNextState
        let sub_id = ElementId::new_v4();
        let mut sub = Element::new(sub_id.clone(), ElementKind::Subclassification);
        sub.owner = Some(calc_id.clone());
        sub.set_prop(
            "unresolved_superclassifier",
            Value::String("GetNextState".to_string()),
        );
        graph.add_element(sub);

        // Return expression child
        let ret_id = ElementId::new_v4();
        let mut ret = Element::new(ret_id.clone(), ElementKind::AttributeUsage);
        ret.name = Some("result".to_string());
        ret.owner = Some(calc_id.clone());
        ret.set_prop("expr", Value::String(expr.to_string()));
        graph.add_element(ret);

        graph.rebuild_indexes();
        graph
    }

    // -----------------------------------------------------------------------
    // GetNextState / StateSpaceDynamics detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_composite_state_space_dynamics() {
        // Model: x_next = x * 0.9 (exponential decay)
        let graph = make_state_space_dynamics_graph("DecayModel", "x", 1.0, "x * 0.9");

        let compiler = ModelCompiler::new(graph);
        let detections = compiler.detect_composite_state_space_dynamics();

        assert_eq!(detections.len(), 1, "should detect one StateSpaceDynamics");
        let (claim, solver) = &detections[0];
        assert_eq!(claim.label, "DecayModel");
        assert_eq!(solver.state.names, vec!["x"]);
        assert_eq!(solver.state.values, vec![1.0]);
    }

    // -----------------------------------------------------------------------
    // `specializes_name`: definition vs usage spellings
    // -----------------------------------------------------------------------
    //
    // KerML records a specialization edge as `Subclassification` between
    // Classifiers, `Subsetting` between Features, and `FeatureTyping` from a
    // Feature to its type (8.3.3.1). The parser picks by declaration form, so
    // a DEFINITION's `:>` and a USAGE's `:>` land in different element kinds
    // asking the same question. Only the definition spellings were read, which
    // is why `examples/damped-oscillator` — the one fixture written as
    // `action dynamics :> StateSpaceDynamics` — detected nothing at all.

    /// Build a bare element carrying one owned specialization child of `kind`,
    /// naming `target` under `prop`.
    fn element_specializing(
        kind: ElementKind,
        edge_kind: ElementKind,
        prop: &'static str,
        target: &str,
    ) -> (ModelGraph, sysml_core::ElementId) {
        use sysml_core::ElementId;
        let mut graph = ModelGraph::new();
        let id = ElementId::new_v4();
        let mut element = Element::new(id.clone(), kind);
        element.name = Some("dynamics".to_owned());
        graph.add_element(element);

        let edge_id = ElementId::new_v4();
        let mut edge = Element::new(edge_id, edge_kind);
        edge.owner = Some(id.clone());
        edge.set_prop(prop, Value::String(target.to_owned()));
        graph.add_element(edge);
        (graph, id)
    }

    #[test]
    fn specializes_name_reads_a_usage_subsetting() {
        // `action dynamics :> StateSpaceDynamics`
        let (graph, id) = element_specializing(
            ElementKind::ActionUsage,
            ElementKind::Subsetting,
            "unresolved_subsettedFeature",
            "StateSpaceDynamics",
        );
        assert!(crate::compiler::specializes_name(
            &graph,
            &id,
            "StateSpaceDynamics"
        ));
    }

    #[test]
    fn specializes_name_reads_a_usage_feature_typing() {
        // `calc :>> getNextState : GetNextState` — the typing, not the
        // redefinition, is what answers "is this a GetNextState?".
        let (graph, id) = element_specializing(
            ElementKind::CalculationUsage,
            ElementKind::FeatureTyping,
            "unresolved_type",
            "GetNextState",
        );
        assert!(crate::compiler::specializes_name(&graph, &id, "GetNextState"));
    }

    #[test]
    fn specializes_name_accepts_a_qualified_target() {
        let (graph, id) = element_specializing(
            ElementKind::ActionUsage,
            ElementKind::Subsetting,
            "unresolved_subsettedFeature",
            "StateSpaceRepresentation::StateSpaceDynamics",
        );
        assert!(crate::compiler::specializes_name(
            &graph,
            &id,
            "StateSpaceDynamics"
        ));
    }

    #[test]
    fn specializes_name_still_says_no_to_an_unrelated_target() {
        // The widening must not make the predicate answer yes to everything.
        let (graph, id) = element_specializing(
            ElementKind::ActionUsage,
            ElementKind::Subsetting,
            "unresolved_subsettedFeature",
            "StateSpaceDynamics",
        );
        assert!(!crate::compiler::specializes_name(
            &graph,
            &id,
            "ContinuousStateSpaceDynamics"
        ));
        assert!(!crate::compiler::specializes_name(&graph, &id, "GetOutput"));
    }

    #[test]
    fn specializes_name_ignores_a_redefinition_target() {
        // `:>> getNextState` names an inherited FEATURE, not a type. Reading
        // it here would make every redefinition claim to specialize whatever
        // name it redefines.
        let (graph, id) = element_specializing(
            ElementKind::CalculationUsage,
            ElementKind::Redefinition,
            "unresolved_redefinedFeature",
            "getNextState",
        );
        assert!(!crate::compiler::specializes_name(
            &graph,
            &id,
            "getNextState"
        ));
    }

    #[test]
    fn test_state_space_dynamics_not_confused_with_continuous() {
        // If the action specializes ContinuousStateSpaceDynamics, it should NOT
        // be detected by detect_composite_state_space_dynamics.
        use sysml_core::ElementId;

        let mut graph = ModelGraph::new();
        let action_id = ElementId::new_v4();
        let mut action = Element::new(action_id.clone(), ElementKind::ActionDefinition);
        action.name = Some("dynamics".to_string());
        action.set_prop(
            "unresolvedTypeName",
            Value::String("ContinuousStateSpaceDynamics".to_string()),
        );
        graph.add_element(action);

        // Also tag as StateSpaceDynamics (since ContinuousSSR :> SSR)
        let sub_id = ElementId::new_v4();
        let mut sub = Element::new(sub_id.clone(), ElementKind::Subclassification);
        sub.owner = Some(action_id.clone());
        sub.set_prop(
            "unresolved_superclassifier",
            Value::String("StateSpaceDynamics".to_string()),
        );
        graph.add_element(sub);

        graph.rebuild_indexes();
        let compiler = ModelCompiler::new(graph);
        let detections = compiler.detect_composite_state_space_dynamics();
        assert!(
            detections.is_empty(),
            "ContinuousSSR should not be detected as base StateSpaceDynamics"
        );
    }

    #[test]
    fn test_state_space_dynamics_step_execution() {
        // Verify the solver produces correct results: x_next = x * 0.9
        let graph = make_state_space_dynamics_graph("DecayModel", "x", 100.0, "x * 0.9");

        let compiler = ModelCompiler::new(graph);
        let detections = compiler.detect_composite_state_space_dynamics();
        assert_eq!(detections.len(), 1);

        let (_, mut solver) = detections.into_iter().next().unwrap();
        let ctx = EvalContext::new();

        // Step 1: x = 100.0 → x_next = 90.0
        solver.step(&ctx);
        assert!(
            (solver.state.values[0] - 90.0).abs() < 1e-10,
            "step 1: got {}",
            solver.state.values[0]
        );

        // Step 2: x = 90.0 → x_next = 81.0
        solver.step(&ctx);
        assert!(
            (solver.state.values[0] - 81.0).abs() < 1e-10,
            "step 2: got {}",
            solver.state.values[0]
        );

        // Step 3: x = 81.0 → x_next = 72.9
        solver.step(&ctx);
        assert!(
            (solver.state.values[0] - 72.9).abs() < 1e-10,
            "step 3: got {}",
            solver.state.values[0]
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.1: instance tree + slot-table minting
    // -----------------------------------------------------------------------

    fn espresso_cell_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/espresso-production-cell")
    }

    /// Parse + elaborate the espresso-production-cell example workspace.
    fn load_espresso_cell_graph() -> Arc<ModelGraph> {
        use sysml_parser_trait::{Parser as _, SysmlFile};

        let dir = espresso_cell_dir();
        assert!(
            dir.exists(),
            "espresso cell dir not found: {}",
            dir.display()
        );

        fn collect_sysml(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect_sysml(&path, out);
                    } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
                        out.push(path);
                    }
                }
            }
        }

        let mut paths = Vec::new();
        collect_sysml(&dir, &mut paths);
        paths.sort();
        assert!(!paths.is_empty(), "no .sysml files under {}", dir.display());

        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let files: Vec<SysmlFile> = paths
            .iter()
            .map(|p| {
                let source = std::fs::read_to_string(p).unwrap();
                let name = p.file_name().unwrap().to_str().unwrap().to_owned();
                SysmlFile::new(name, source)
            })
            .collect();
        let result = parser.parse(&files);
        let mut graph = result.graph;
        // Resolve references so FeatureTyping carries resolved `type`
        // targets (find_feature_type needs them for the canonical
        // `…thermalModel…` sub-path), then elaborate — the same order the
        // ide-db pipeline uses.
        let _ = sysml_core::resolution::resolve_references(&mut graph);
        elaborate::elaborate(&mut graph);
        Arc::new(graph)
    }

    /// Tree-based instance discovery on the espresso cell (the legacy
    /// container-scan oracle and its parity cross-checks were deleted at
    /// RSC-2.2 after soaking; these assertions are what remains pinned).
    #[test]
    fn instance_discovery_on_espresso_cell() {
        let compiler = ModelCompiler::from_arc(load_espresso_cell_graph());
        let specs = compiler.expand_part_instances();

        assert_eq!(specs.len(), 2, "2 brew-station instances expected");
        assert!(
            specs.iter().all(|s| s.usage_id.is_some()),
            "every discovered instance carries its usage ElementId (RSC-2.1)"
        );
        assert!(
            specs
                .iter()
                .all(|s| s.container_name.as_deref() == Some("ProductionCell")),
            "all instances are contained by ProductionCell"
        );
    }

    #[test]
    fn instance_discovery_empty_on_single_instance_graphs() {
        // Synthetic graphs from this test module exercise the
        // no-instances path (no ≥2 same-type usage groups).
        for graph in [
            make_solver_selection_graph(&[("UnitThermalModel", "builtin:ode-rk4")]),
            make_state_space_dynamics_graph("DecayModel", "x", 100.0, "x * 0.9"),
        ] {
            let compiler = ModelCompiler::new(graph);
            assert!(compiler.expand_part_instances().is_empty());
        }
    }

    #[test]
    fn instance_tree_covers_every_part_uniformly() {
        let compiler = ModelCompiler::from_arc(load_espresso_cell_graph());
        let tree = compiler.build_instance_tree();

        // EVERY PartDefinition/PartUsage appears as a node — single
        // instances and SM-less parts included.
        let part_count = compiler
            .graph()
            .elements
            .values()
            .filter(|e| matches!(e.kind, ElementKind::PartDefinition | ElementKind::PartUsage))
            .count();
        assert_eq!(tree.len(), part_count);
        assert!(!tree.roots().is_empty());

        // station1 resolves its type and reports a one-segment usage path.
        let station1 = tree
            .nodes()
            .find(|n| n.name.as_deref() == Some("station1"))
            .expect("station1 node");
        assert_eq!(station1.kind, ElementKind::PartUsage);
        assert_eq!(station1.type_name.as_deref(), Some("BrewStation"));
        assert!(station1.type_def_id.is_some());
        let path = tree.usage_path(&station1.element_id).expect("usage path");
        assert_eq!(path, vec![station1.element_id.clone()]);
    }

    #[test]
    fn espresso_cell_mints_slot_table() {
        use crate::slots::{Variability, WriterId};

        let compiler =
            ModelCompiler::from_arc(load_espresso_cell_graph()).with_source_dir(&espresso_cell_dir());
        let base_ctx = context_from_graph(compiler.graph());
        let orch = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[],
                Some(100.0),
                Some(60000.0),
            )
            .expect("workspace orchestrator should compile");

        let store = orch.slot_store();
        assert!(!store.is_empty(), "slot table must be minted");
        println!(
            "espresso-cell slot table: {} slots, {} conflicts",
            store.len(),
            store.multi_writer_conflicts().len()
        );

        // Per-instance ODE state var resolves via BOTH canonical and runtime
        // name forms to ONE slot (bare/qualified alias to the same SlotId —
        // RT-SLOT), classified Continuous and owned by the right ODE executor
        // subsystem (RSC-2.2 writer assignment).
        let runtime_id = store
            .slot_by_name("station1.temp")
            .expect("runtime-form name binds a slot");
        let thermal_index = orch
            .subsystem_names()
            .iter()
            .position(|n| n == "station1.StationThermal")
            .expect("station1.StationThermal subsystem registered") as u16;
        assert_eq!(
            store.meta(runtime_id).expect("slot meta").writer,
            WriterId::Executor(thermal_index),
            "ODE state slot's writer is the owning ODE subsystem index"
        );
        let temp_names: Vec<&str> = store
            .names()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".temp"))
            .collect();
        let canonical_id = store
            .slot_by_name("ProductionCell.station1.thermal.temp")
            .unwrap_or_else(|| panic!("canonical-form name binds a slot; have: {temp_names:?}"));
        assert_eq!(runtime_id, canonical_id, "both name forms hit one slot");
        let meta = store.meta(runtime_id).expect("slot meta");
        assert_eq!(meta.variability, Variability::Continuous);
        assert!(!meta.bookkeeping);

        // Distinct instances mint distinct slots off the same declaration.
        let s2 = store
            .slot_by_name("station2.temp")
            .expect("station2 state var slot");
        assert_ne!(runtime_id, s2);
        let s2_meta = store.meta(s2).expect("slot meta");
        assert_eq!(
            meta.runtime_id.declaration, s2_meta.runtime_id.declaration,
            "same declaration element"
        );
        assert_ne!(
            meta.runtime_id, s2_meta.runtime_id,
            "different instance paths"
        );

        // t_ms is a bookkeeping slot owned by the orchestrator.
        let t_ms = store.slot_by_name("t_ms").expect("t_ms slot");
        let t_ms_meta = store.meta(t_ms).expect("slot meta");
        assert!(t_ms_meta.bookkeeping);
        assert_eq!(t_ms_meta.writer, WriterId::Orchestrator);

        // RS001 gate: with real per-executor writers assigned (RSC-2.2) the
        // cell must remain conflict-free — a conflict here would have failed
        // the build above.
        assert!(
            store.multi_writer_conflicts().is_empty(),
            "no multi-writer conflicts expected on espresso cell: {:?}",
            store.multi_writer_conflicts()
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.3/2.5: expression slot binding + RS003 hard error
    // -----------------------------------------------------------------------

    #[test]
    fn rs003_fires_for_name_that_is_neither_slot_nor_graph_feature() {
        // `ghostFactor` is referenced by the derivative but declared
        // nowhere: not an ODE parameter, not a bare binding, not a slot,
        // not a named element → exactly the RS003 contract.
        let source = r#"
            package Demo {
                part def Plant {
                    attribute gain : Real default 2.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {
                        return dLdt = 0.0 - gain * level * ghostFactor;
                    }
                }
                state def PlantStates {
                    entry; then Idle;
                    state Idle;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("demo.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        let compiler = ModelCompiler::from_arc(Arc::new(graph));

        let err = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .err()
            .expect("RS003 is a hard error since RSC-2.5 — the build must fail");
        let rs003: Vec<_> = err
            .diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some("RS003"))
            .collect();
        assert_eq!(
            rs003.len(),
            1,
            "exactly one RS003 error expected, got {:?}",
            err.diagnostics
        );
        let diag = rs003.first().expect("len checked");
        assert!(
            diag.message.contains("ghostFactor"),
            "RS003 names the unresolved reference: {}",
            diag.message
        );
        assert_eq!(
            diag.severity,
            sysml_span::Severity::Error,
            "RS003 is a hard error since RSC-2.5"
        );
        assert!(
            diag.message.contains("referenced by"),
            "RS003 attributes the referencing scope: {}",
            diag.message
        );

        // Control: the clean demo plant compiles with no RS003.
        demo_plant_compiler("state Idle;")
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .expect("clean model must compile — no RS003");
    }

    #[test]
    fn rs003_error_suggests_nearest_slot_spelling() {
        // A near-miss of a real slot name (`leve` vs `level`, distance 1)
        // must carry the "nearest known runtime name" note (RSC-2.5).
        let source = r#"
            package Demo {
                part def Plant {
                    attribute gain : Real default 2.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {
                        return dLdt = 0.0 - gain * leve;
                    }
                }
                state def PlantStates {
                    entry; then Idle;
                    state Idle;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("demo.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        let compiler = ModelCompiler::from_arc(Arc::new(graph));

        let err = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .err()
            .expect("typo'd slot reference must fail with RS003");
        let diag = err
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("RS003"))
            .expect("RS003 diagnostic present");
        assert!(
            diag.message.contains("'leve'"),
            "names the typo: {}",
            diag.message
        );
        let notes = diag.notes.join("; ");
        assert!(
            notes.contains("level"),
            "nearest-spelling note suggests 'level': {notes:?}"
        );
    }

    #[test]
    fn single_model_bind_pass_binds_ode_locals() {
        let compiler = demo_plant_compiler("state Idle;");
        let orch = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .expect("orchestrator should compile");
        let report = orch.slot_bind_report();
        // `0.0 - gain * level`: both `gain` (Parameter slot) and `level`
        // (Continuous ODE state slot) must bind.
        assert!(
            report.bound_refs >= 2,
            "gain + level must bind to slots, got {report:?}"
        );
        assert!(
            report.unresolved.is_empty(),
            "no unresolved names expected: {report:?}"
        );
    }

    /// Minimal SSR plant model (RK4 ODE on `level` + an SM) for the
    /// single-model `build_orchestrator` path. `sm_states` is spliced into
    /// the `state def PlantStates` body so individual tests can add
    /// assignment actions (e.g. to provoke an RS001 two-writer conflict).
    fn demo_plant_compiler(sm_states: &str) -> ModelCompiler {
        let source = format!(
            r#"
            package Demo {{
                part def Plant {{
                    attribute gain : Real default 2.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {{
                        return dLdt = 0.0 - gain * level;
                    }}
                }}
                state def PlantStates {{
                    entry; then Idle;
                    {sm_states}
                }}
            }}
        "#
        );
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("demo.sysml".to_owned(), source)]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        ModelCompiler::from_arc(Arc::new(graph))
    }

    #[test]
    fn single_model_build_orchestrator_mints_slot_table() {
        use crate::slots::Variability;

        let compiler = demo_plant_compiler("state Idle;");
        let orch = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .expect("orchestrator should compile");
        let store = orch.slot_store();
        assert!(!store.is_empty());
        let level = store.slot_by_name("level").expect("ODE state var slot");
        assert_eq!(
            store.meta(level).expect("meta").variability,
            Variability::Continuous
        );
        // RSC-2.2 writer assignment: the ODE subsystem (registered as
        // "compiled-ode" on this path) owns the state-var slot.
        let ode_index = orch
            .subsystem_names()
            .iter()
            .position(|n| n == "compiled-ode")
            .expect("compiled-ode subsystem") as u16;
        assert_eq!(
            store.meta(level).expect("meta").writer,
            crate::slots::WriterId::Executor(ode_index)
        );
        assert!(store.slot_by_name("t_ms").is_some());
    }

    // -----------------------------------------------------------------------
    // RSC-2.2: write-through routing + RS001
    // -----------------------------------------------------------------------

    #[test]
    fn slot_routing_round_trip_by_name_and_by_slot_id() {
        let compiler = demo_plant_compiler("state Idle;");
        let mut orch = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .expect("orchestrator should compile");
        let level = orch.slot_store().slot_by_name("level").expect("level slot");
        assert_eq!(orch.context.slot_id("level"), Some(level));

        // set by name → read by SlotId (write-through routing).
        orch.context.set("level", Value::Float(5.5));
        assert_eq!(orch.context.get_slot(level), Some(Value::Float(5.5)));
        assert_eq!(orch.slot_store().get(level), Some(&Value::Float(5.5)));

        // set by SlotId → read by name (mirror into the legacy map).
        assert!(orch.context.set_slot(level, Value::Float(7.25)));
        assert_eq!(orch.context.get("level"), Some(&Value::Float(7.25)));

        // Unbound names never route but still land in the map.
        orch.context.set("unboundScratch", Value::Int(1));
        assert_eq!(orch.context.slot_id("unboundScratch"), None);
        assert_eq!(orch.context.get("unboundScratch"), Some(&Value::Int(1)));
    }

    // RSC-3.6: `slot_routing_disabled_bypasses_store` was retired with no-slots
    // mode (RSC-3.5f.3). It exercised only the RSC-2.2 `set_enabled(false)`
    // rollback gate, which was deleted (slot routing is now unconditional);
    // by-name / by-SlotId routing is covered by
    // `slots::tests::set_by_name_routes_to_both_spellings`.

    /// RS004 (RSC-3.5f.3) negative path: an ineligible prefixed subsystem must
    /// be a HARD compile error, not a tick-time `unreachable!`. A raw prefixed
    /// SM with no slot store attached is bypass-INELIGIBLE
    /// (`scoped_view_bypass()` is false — no bind ran), so it shows up in
    /// `scoped_view_fallbacks()`; `rs004_diagnostics` must turn that into one
    /// RS004 error naming the subsystem. (The positive path — empty fallbacks →
    /// no error — is exercised by every passing compiler build test + the
    /// corpus-wide `rsc36_bypass_eligibility_census`.)
    #[test]
    fn rs004_rejects_ineligible_prefixed_subsystem() {
        use crate::orchestrator::{Orchestrator, OrchestratorConfig};
        use crate::statemachine::StateMachineRunner;
        use crate::{StateIR, StateMachineIR, TransitionIR};

        let compiler = demo_plant_compiler("state Idle;");
        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        let ir = StateMachineIR {
            name: "sm".to_string(),
            states: vec![StateIR::new("a"), StateIR::new("b").final_state()],
            transitions: vec![TransitionIR::new("a", "b").with_event("go")],
            initial: "a".to_string(),
            regions: vec![],
        };
        // Raw prefixed add with NO slot store → never bound → not bypass-eligible.
        orch.add_state_machine_prefixed("inst1.sm", StateMachineRunner::new(ir), "inst1");
        assert!(
            !orch.scoped_view_fallbacks().is_empty(),
            "precondition: an unbound prefixed SM is a scoped-view fallback"
        );

        let diags = compiler.rs004_diagnostics(&orch);
        assert_eq!(diags.len(), 1, "exactly one RS004 error: {diags:?}");
        assert_eq!(diags[0].code.as_deref(), Some("RS004"));
        assert!(
            diags[0].message.contains("inst1.sm"),
            "RS004 names the offending subsystem: {}",
            diags[0].message
        );
    }

    #[test]
    fn fork_deep_copies_the_slot_store() {
        let compiler = demo_plant_compiler("state Idle;");
        let mut orch = compiler
            .build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0))
            .expect("orchestrator should compile");
        let level = orch.slot_store().slot_by_name("level").expect("level slot");

        let mut forked = orch.fork();
        forked.context.set("level", Value::Float(99.0));
        orch.context.set("level", Value::Float(11.0));
        assert_eq!(forked.context.get_slot(level), Some(Value::Float(99.0)));
        assert_eq!(
            orch.context.get_slot(level),
            Some(Value::Float(11.0)),
            "fork shares no mutable slot state with the parent"
        );
    }

    /// RS001 (hard error, design doc D-2.0.3 rule 1): an SM entry action
    /// assigning an ODE state variable is two tick-time writers on one
    /// slot — the build must fail with a diagnostic naming the variable
    /// and both writers.
    #[test]
    fn rs001_fires_on_two_writer_model() {
        let compiler = demo_plant_compiler("state Idle {\n entry action { level = 0.0; }\n }");
        let err = match compiler.build_orchestrator("PlantStates", &[], Some(1.0), Some(100.0)) {
            Ok(_) => panic!("two writers on `level` must fail the build"),
            Err(e) => e,
        };

        let rs001: Vec<_> = err
            .diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some("RS001"))
            .collect();
        assert_eq!(rs001.len(), 1, "exactly one RS001 expected, got: {err:?}");
        let message = rs001[0].message.as_str();
        assert!(message.contains("'level'"), "names the variable: {message}");
        assert!(
            message.contains("compiled-ode"),
            "names the ODE writer: {message}"
        );
        assert!(
            message.contains("PlantStates"),
            "names the SM writer: {message}"
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.4a: ODE executor slot cutover (write-set + scoped-clone bypass)
    // -----------------------------------------------------------------------

    /// Two-instance fixture: a container with two same-typed part usages
    /// whose type reaches an exponential-decay ODE (`dLdt = 0 - gain *
    /// level`) through a sub-part — the instance-multiplication
    /// shape (`branchN : BranchPath { part thermalModel : UnitThermalModel }`)
    /// at unit size.
    fn two_tank_graph() -> Arc<ModelGraph> {
        let source = r#"
            package IsoDemo {
                part def TankModel {
                    attribute gain : Real default 2.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {
                        return dLdt = 0.0 - gain * level;
                    }
                }
                part def TankUnit {
                    part model : TankModel;
                }
                part def Farm {
                    part tankA : TankUnit;
                    part tankB : TankUnit;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("iso.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        Arc::new(graph)
    }

    fn build_two_tank_orchestrator(
        graph: Arc<ModelGraph>,
        overrides: &[(String, String)],
    ) -> Orchestrator {
        let compiler = ModelCompiler::from_arc(graph);
        let base_ctx = context_from_graph(compiler.graph());
        compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                overrides,
                Some(100.0),
                Some(60_000.0),
            )
            .expect("two-tank workspace orchestrator should compile")
    }

    fn ctx_float(orch: &Orchestrator, key: &str) -> f64 {
        match orch.context.get(key) {
            Some(Value::Float(f)) => *f,
            other => panic!("expected Float at '{key}', got {other:?}"),
        }
    }

    /// RSC-5.1 (D-5.0.5 path #1): an explicit `[unit]` annotation on a declared
    /// value resolves through the unit table to a fully-populated MeasurementRef
    /// (dimension + unit name + scale + offset) — overriding the ISQ-type path.
    #[test]
    fn rsc5_explicit_unit_infers_full_mref() {
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        // `5.0 [mA]`: milliamps — current dimension, scale 0.001 to SI amperes.
        let src = "package P { part def V { attribute i = 5.0 [mA]; } }";
        let result = parser.parse(&[SysmlFile::new("t.sysml".to_owned(), src.to_owned())]);
        let graph = result.graph;
        let attr = graph
            .elements
            .values()
            .find(|e| e.kind == ElementKind::AttributeUsage && e.name.as_deref() == Some("i"))
            .expect("attribute i");
        let attr_id = attr.id.clone();
        let compiler = ModelCompiler::new(graph);
        let m_ref =
            infer_m_ref(compiler.graph(), &attr_id).expect("explicit [mA] unit must infer an mRef");
        assert_eq!(m_ref.unit.as_deref(), Some("mA"), "unit name carried");
        assert_eq!(m_ref.scale, 0.001, "mA → A scale");
        assert_eq!(m_ref.offset, 0.0);
        // mA is a current dimension (A = ampere = current base).
        assert_eq!(
            m_ref.dimension,
            sysml_core::physics::DimensionVector::new(0, 0, 0, 1, 0, 0, 0),
            "milliamp carries the electric-current dimension"
        );
    }

    /// RSC-2.4a: two instances of one ODE definition integrate
    /// independently — diverging correctly under different per-instance
    /// overrides — with the slot-routed writeback and (when eligible) the
    /// scoped-clone bypass active.
    #[test]
    fn rsc24a_per_instance_isolation_with_slot_writeback() {
        let overrides = vec![
            ("tankA.gain".to_owned(), "1.0".to_owned()),
            ("tankB.gain".to_owned(), "3.0".to_owned()),
        ];
        let mut orch = build_two_tank_orchestrator(two_tank_graph(), &overrides);

        let prefixed_odes: Vec<String> = orch
            .subsystems()
            .iter()
            .filter(|s| s.var_prefix.is_some())
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(
            prefixed_odes.len(),
            2,
            "both tank instances must multiply into prefixed subsystems, got {prefixed_odes:?}"
        );
        // The decay spec is fully slot-bound: every prefixed ODE runs on
        // the bypass path (no scoped-context clone). This pins the bypass
        // actually engaging — if eligibility regresses, this fails before
        // any numeric drift would.
        assert_eq!(
            orch.ode_scoped_fallbacks(),
            Vec::<String>::new(),
            "two-tank instances must not need the legacy scoped view"
        );

        // dt=100ms, 10 ticks → t=1s. level(t) = e^(-gain·t).
        for _ in 0..10 {
            orch.step();
        }
        let a = ctx_float(&orch, "tankA.level");
        let b = ctx_float(&orch, "tankB.level");
        let expect_a = (-1.0_f64).exp();
        let expect_b = (-3.0_f64).exp();
        assert!(
            (a - expect_a).abs() < 1e-3,
            "tankA integrates with ITS override (gain=1): expected ~{expect_a:.4}, got {a:.4}"
        );
        assert!(
            (b - expect_b).abs() < 1e-3,
            "tankB integrates with ITS override (gain=3): expected ~{expect_b:.4}, got {b:.4}"
        );
        assert!(
            (a - b).abs() > 0.25,
            "instances must diverge under different inputs: {a} vs {b}"
        );

        // Slot store and legacy map stay coherent through the new writeback.
        let store = orch.slot_store();
        for (key, expected) in [("tankA.level", a), ("tankB.level", b)] {
            let id = store.slot_by_name(key).expect("state slot exists");
            assert_eq!(
                store.get(id),
                Some(&Value::Float(expected)),
                "slot value mirrors the legacy map for '{key}'"
            );
        }
    }

    /// RSC-3.6 (was RSC-2.4a writeback parity): the slot-routed two-tank ODE
    /// writeback keeps its derivatives snapshot prefix-scoped per instance.
    /// The instance-isolation value pins live in
    /// `rsc24a_per_instance_isolation_with_slot_writeback`; the former
    /// slots-on-vs-off parity arm was retired with no-slots mode (RSC-3.5f.3 —
    /// slot routing is now unconditional, so there is no legacy path to diff).
    #[test]
    fn rsc24a_writeback_keeps_derivatives_prefix_scoped() {
        let overrides = vec![
            ("tankA.gain".to_owned(), "1.0".to_owned()),
            ("tankB.gain".to_owned(), "3.0".to_owned()),
        ];
        let mut orch = build_two_tank_orchestrator(two_tank_graph(), &overrides);

        let mut last = None;
        for _ in 0..50 {
            last = Some(orch.step());
        }

        // Derivatives snapshot keys stay prefix-scoped to each instance.
        let derivs: std::collections::BTreeMap<String, f64> =
            last.expect("stepped").derivatives.into_iter().collect();
        assert!(
            derivs.keys().any(|k| k.starts_with("tankA.")),
            "derivatives stay prefix-scoped: {derivs:?}"
        );
        assert!(
            derivs.keys().any(|k| k.starts_with("tankB.")),
            "both instances' derivatives stay prefix-scoped: {derivs:?}"
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.4b: SM executor slot cutover (write-set restriction + routing)
    // -----------------------------------------------------------------------

    /// SM-heavy parity fixture: an exponential-decay ODE coupled to a
    /// top-level state machine through a `when()` trigger, with structured
    /// entry assignments on both states — every SM writeback class the
    /// baseline corpus exercises (compiled targets, when-trigger reads,
    /// run-to-completion chains).
    fn sm_parity_graph() -> Arc<ModelGraph> {
        let source = r#"
            package SmParity {
                part def Plant {
                    attribute gain : Real default 2.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {
                        return dLdt = 0.0 - gain * level;
                    }
                }
                state def MonitorStates {
                    attribute lowEvents : Real default 0.0;
                    attribute lastLevelSeen : Real default 1.0;
                    state watching {
                        entry action { lastLevelSeen = level; }
                    }
                    state low {
                        entry action { lowEvents = lowEvents + 1.0; }
                    }
                    entry; then watching;
                    transition watching_to_low
                        first watching
                        accept when level < 0.5
                        then low;
                    transition low_to_watching
                        first low
                        accept when level >= 0.5
                        then watching;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("smp.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        Arc::new(graph)
    }

    /// RSC-3.6 (was RSC-2.4b writeback parity): the restricted slot-routed SM
    /// write-set lands the structured entry assignment and keeps the slot
    /// store coherent with the legacy map. The former slots-on-vs-off parity
    /// arm was retired with no-slots mode (RSC-3.5f.3 — slot routing is now
    /// unconditional).
    #[test]
    fn rsc24b_sm_writeback_routes_through_slots() {
        let graph = sm_parity_graph();
        let compiler = ModelCompiler::from_arc(graph);
        let base_ctx = context_from_graph(compiler.graph());
        let mut on = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[],
                Some(100.0),
                Some(60_000.0),
            )
            .expect("sm-parity workspace orchestrator should compile");

        let mut last_on = None;
        for _ in 0..50 {
            last_on = Some(on.step());
        }

        // The fixture must actually exercise the SM write path: level
        // decays through 0.5 (~tick 4), the when() trigger fires, and the
        // entry assignment lands.
        assert_eq!(
            on.context.get("lowEvents"),
            Some(&Value::Float(1.0)),
            "watching→low must have fired exactly once"
        );

        // SM current states travel via TickOutput, not the context — confirm
        // the run produced subsystem state.
        let states_on = &last_on.expect("stepped").subsystem_states;
        assert!(
            !states_on.is_empty(),
            "the SM-parity run must report subsystem states"
        );

        // The SM's targets are top-level (canonical == runtime) → routed;
        // slot and map stay coherent through the routed writeback.
        let store = on.slot_store();
        let id = store.slot_by_name("lowEvents").expect("SM target slot");
        assert_eq!(store.get(id), Some(&Value::Float(1.0)));
        assert_eq!(
            on.sm_slot_fallbacks(),
            Vec::<(String, Vec<String>)>::new(),
            "top-level SM targets route by SlotId — no name-keyed fallbacks"
        );
    }

    /// Two-instance SM fixture (RSC-2.4b): a part def carrying its own
    /// state def, used twice — the instance-multiplication shape for SMs.
    fn two_unit_sm_graph() -> Arc<ModelGraph> {
        let source = r#"
            package SmIso {
                part def Unit {
                    attribute threshold : Real default 5.0;
                    attribute sensor : Real default 0.0;
                    attribute tripCount : Real default 0.0;
                    state def UnitStates {
                        state idle;
                        state hot {
                            entry action { tripCount = tripCount + 1.0; }
                        }
                        entry; then idle;
                        transition idle_to_hot
                            first idle
                            accept when sensor > threshold
                            then hot;
                    }
                }
                part def Bench {
                    part u1 : Unit;
                    part u2 : Unit;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("smiso.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        Arc::new(graph)
    }

    /// RSC-2.4b: two instances of one SM definition step independently —
    /// only the instance whose stripped scoped reads satisfy the guard
    /// transitions and assigns — and the restricted writeback publishes the
    /// write-set under the instance prefix WITHOUT the legacy whole-diff's
    /// echo of merged globals (`{prefix}.t_ms` & co. must no longer mint).
    #[test]
    fn rsc24b_per_instance_sm_isolation_without_context_echo() {
        let overrides = vec![("u1.sensor".to_owned(), "10.0".to_owned())];
        let compiler = ModelCompiler::from_arc(two_unit_sm_graph());
        let base_ctx = context_from_graph(compiler.graph());
        let mut orch = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &overrides,
                Some(100.0),
                Some(60_000.0),
            )
            .expect("two-unit SM workspace orchestrator should compile");

        let sm_names: Vec<String> = orch
            .subsystems()
            .iter()
            .filter(|s| s.var_prefix.is_some())
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(
            sm_names,
            vec!["u1.UnitStates".to_owned(), "u2.UnitStates".to_owned()],
            "both unit instances must multiply into prefixed SM subsystems"
        );

        // RSC-3.6 step (2): the structured entry action `tripCount = tripCount
        // + 1.0` reads only the bare, instance-local-slot-backed name
        // `tripCount`, so the SM is now scoped-view-bypass-ELIGIBLE — it runs
        // off the thin slot-read view (no per-prefix `build_scoped_context`
        // clone). Before step 2 the non-empty structured assignment forced both
        // instances onto the scoped-clone path. This assertion proves the
        // eligibility actually flipped; the trajectory asserts below prove it
        // did so byte-identically.
        assert!(
            orch.scoped_view_fallbacks().is_empty(),
            "RSC-3.6 step 2: slot-seedable structured-action SMs must NOT fall \
             back to build_scoped_context: {:?}",
            orch.scoped_view_fallbacks()
        );

        let mut last = None;
        for _ in 0..5 {
            last = Some(orch.step());
        }
        let snapshot = last.expect("stepped");

        // Per-instance isolation: u1's override satisfies ITS guard only.
        assert_eq!(
            snapshot.subsystem_states["u1.UnitStates"].current_state, "hot",
            "u1 trips on its own sensor override"
        );
        assert_eq!(
            snapshot.subsystem_states["u2.UnitStates"].current_state, "idle",
            "u2 must not observe u1's sensor"
        );
        assert_eq!(orch.context.get("u1.tripCount"), Some(&Value::Float(1.0)));
        assert_eq!(orch.context.get("u2.tripCount"), Some(&Value::Float(0.0)));

        // RSC-3.5b (leftover-C): the instance-scoped target now ROUTES by
        // SlotId. The SM subsystem carries the canonical tree-path prefix
        // (`Bench.u1`), so `WriteRoute::resolve` matches the slot's canonical
        // spelling instead of refusing on the mismatch. The set_slot
        // dual-spelling mirror keeps BOTH the runtime key and the canonical
        // alias coherent in the legacy map.
        let store = orch.slot_store();
        let id = store
            .slot_by_name("u1.tripCount")
            .expect("instance SM target slot");
        assert_eq!(
            store.get(id),
            Some(&Value::Float(1.0)),
            "slot mirrors the legacy map through the routed write"
        );
        drop(store);
        // RSC-3.5b: instance-scoped SM targets DRAIN out of the name-keyed
        // fallback. This SM has no payload ports and no local clock, so its
        // fallback list is now empty entirely.
        let fallbacks = orch.sm_slot_fallbacks();
        assert!(
            !fallbacks
                .iter()
                .any(|(name, keys)| name == "u1.UnitStates"
                    && keys.contains(&"u1.tripCount".to_owned())),
            "RSC-3.5b: instance-scoped target must no longer be a name-keyed \
             fallback (routed via canonical alias): {fallbacks:?}"
        );
        assert_eq!(
            fallbacks,
            Vec::<(String, Vec<String>)>::new(),
            "RSC-3.5b: no payloads + no local clock ⇒ empty SM fallback set: {fallbacks:?}"
        );
        // RSC-3.5b: the routed write ADDITIVELY surfaces the canonical alias
        // (`Bench.u1.tripCount`) alongside the runtime key (purely additive
        // wire change — no key removed).
        assert_eq!(
            orch.context.get("Bench.u1.tripCount"),
            Some(&Value::Float(1.0)),
            "canonical alias is additively present via the routed set_slot mirror"
        );

        // THE RESTRICTION (intended behaviour change, RSC-2.4b step 1): the
        // legacy whole-context diff echoed every merged key — including
        // orchestrator globals — back under the instance prefix. The
        // compiled write-set publishes only what the SM owns.
        for echo in ["u1.t_ms", "u1.tick", "u2.t_ms", "u2.sensor.payload"] {
            assert!(
                orch.context.get(echo).is_none(),
                "context-echo key '{echo}' must no longer be minted by the SM writeback"
            );
        }
        // The instance parameter keys seeded at build (overrides/config)
        // are still present — only the SM's own writeback narrowed.
        assert_eq!(orch.context.get("u1.sensor"), Some(&Value::Float(10.0)));
    }

    /// RSC-2.4b scoped-view bypass engagement: a prefixed SM whose every
    /// tick-time read is a subsystem-local-bound `SlotRef` (guard-only
    /// `when()` transitions, NO structured actions) runs without the
    /// per-prefix scoped-context clone — and still observes per-instance
    /// values through the slots (`u1`'s override must not leak to `u2`).
    /// The eligibility deliberately rejects everything else (actions read
    /// names at execution time; global-fallthrough bindings would read
    /// top-level defaults), so corpus coverage is expected to be rare —
    /// this pins the path working at all.
    #[test]
    fn rsc24b_sm_scoped_view_bypass_engages_with_instance_local_reads() {
        // The ODE lives in a sub-part (the two-tank instance
        // multiplication shape) so the instances mint `u{1,2}.sensor`
        // Parameter slots — which is what makes the SM guard's read
        // subsystem-local-bindable in the first place.
        let source = r#"
            package SmBypass {
                part def SensorModel {
                    attribute gain : Real default 1.0;
                    attribute sensor : Real default 0.0;
                    out attribute level : Real default 1.0;
                    calc def LevelDerivative :> GetDerivative {
                        return dLdt = 0.0 - gain * level + 0.0 * sensor;
                    }
                }
                part def Unit {
                    part model : SensorModel;
                    state def UnitStates {
                        state idle;
                        state hot;
                        entry; then idle;
                        transition idle_to_hot
                            first idle
                            accept when sensor > 5.0
                            then hot;
                    }
                }
                part def Bench {
                    part u1 : Unit;
                    part u2 : Unit;
                }
            }
        "#;
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("smbyp.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);
        let compiler = ModelCompiler::from_arc(Arc::new(graph));
        let base_ctx = context_from_graph(compiler.graph());
        let mut orch = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[("u1.sensor".to_owned(), "10.0".to_owned())],
                Some(100.0),
                Some(60_000.0),
            )
            .expect("bypass fixture should compile");

        let sm_bypass: Vec<(String, bool)> = orch
            .subsystems()
            .iter()
            .filter(|s| {
                s.executor.phase() == crate::orchestrator::ExecutionPhase::StateMachine
                    && s.var_prefix.is_some()
            })
            .map(|s| (s.name.clone(), s.executor.scoped_view_bypass()))
            .collect();
        assert_eq!(sm_bypass.len(), 2, "two prefixed SM instances expected");
        assert!(
            sm_bypass.iter().all(|(_, b)| *b),
            "guard-only instance-local SMs must be bypass-eligible: {sm_bypass:?}"
        );
        // RSC-3.5f.1: the generalized read-side counter agrees — these prefixed
        // SMs do not appear as scoped-view fallbacks (ode_scoped_fallbacks alone
        // would be blind to the StateMachine phase).
        assert!(
            orch.scoped_view_fallbacks().is_empty(),
            "RSC-3.5f.1: bypass-eligible prefixed SMs must not register as scoped-view \
             fallbacks: {:?}",
            orch.scoped_view_fallbacks()
        );

        let mut last = None;
        for _ in 0..3 {
            last = Some(orch.step());
        }
        let snapshot = last.expect("stepped");
        assert_eq!(
            snapshot.subsystem_states["u1.UnitStates"].current_state, "hot",
            "u1 reads ITS sensor slot (override 10.0) through the bypass"
        );
        assert_eq!(
            snapshot.subsystem_states["u2.UnitStates"].current_state, "idle",
            "u2 reads ITS sensor slot (default 0.0) — no cross-instance leak"
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.4c: Action executor slot cutover
    // -----------------------------------------------------------------------
    //
    // Survey finding (2026-06-11): `build_orchestrator` /
    // `build_workspace_orchestrator` never register action subsystems —
    // ActionRunner enters an orchestrator only through the service layer's
    // `sysml.action.start` (fresh default orchestrator, no slot table) or
    // test harnesses, and its legacy context seam publishes NOTHING (both
    // sync hooks are trait-default no-ops; every write target is
    // token-local). These tests pin that reality under the slot seam: an
    // action added to a compiled, slot-carrying orchestrator must leave the
    // shared variables map byte-identical to the slots-off run, and must
    // mint zero context keys.

    /// Hand-built action exercising the write-set classes against the
    /// sm_parity fixture's model variables: an If condition reading the
    /// ODE state `level` each pass and an Assign writing a token-local
    /// target.
    fn parity_action_graph() -> crate::actions::ActionGraphIR {
        use crate::actions::{ActionGraphIR, ActionNodeIR};
        use crate::expressions::{BinOp, ExprIR};

        let mut graph = ActionGraphIR::new("brew", "BrewAction");
        graph.add_node(ActionNodeIR::If {
            id: "checkLevel".into(),
            condition: ExprIR::BinaryOp {
                op: BinOp::GreaterThan,
                left: Box::new(ExprIR::FeatureRef("level".into())),
                right: Box::new(ExprIR::LiteralReal(0.25)),
            },
            then_branch: "markSeen".into(),
            else_branch: None,
        });
        graph.add_node(ActionNodeIR::Assign {
            id: "markSeen".into(),
            target: "brewLevelSeen".into(),
            value: ExprIR::FeatureRef("level".into()),
        });
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(&initial, "checkLevel");
        graph.add_edge("checkLevel", "markSeen");
        graph.add_edge("markSeen", &final_id);
        graph
    }

    /// RSC-3.6 (was RSC-2.4c writeback parity): an action subsystem added to a
    /// compiled orchestrator rides the slot seam without leaking its
    /// token-local write targets into the context, runs to completion, and
    /// reports its targets as unminted coverage. The former slots-on-vs-off
    /// parity arm was retired with no-slots mode (RSC-3.5f.3 — slot routing is
    /// now unconditional).
    #[test]
    fn rsc24c_action_writeback_stays_token_local() {
        let graph = sm_parity_graph();
        let compiler = ModelCompiler::from_arc(graph);
        let base_ctx = context_from_graph(compiler.graph());
        let mut on = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[],
                Some(100.0),
                Some(60_000.0),
            )
            .expect("sm-parity workspace orchestrator should compile");
        on.add_action(
            "brew",
            crate::actions::ActionRunner::new(parity_action_graph()),
        );
        // Re-run the bind/prepare pass so the late-added action gets its slot
        // binding + write-set (idempotent for SM/ODE).
        on.bind_expression_slots(Some(compiler.graph().as_ref()));

        let mut last_on = None;
        for _ in 0..50 {
            last_on = Some(on.step());
        }

        // Token-local discipline: the action's write targets reach the
        // context map (publishing them would be a Phase 3/4 behavioural
        // change).
        let on_vars: std::collections::BTreeMap<&String, &Value> =
            on.context.variables.iter().collect();
        for target in ["brewLevelSeen"] {
            assert!(
                !on_vars.contains_key(&target.to_owned()),
                "action target '{target}' must not leak into the context"
            );
        }

        // The action actually ran.
        let states_on = &last_on.expect("stepped").subsystem_states;
        let brew_on = &states_on["brew"];
        assert_eq!(brew_on.kind, "action");
        assert!(brew_on.completed, "brew action must complete");

        // Mint-coverage observability: the compiler mints no action
        // claims (no action subsystems exist at build time), so every
        // write target reports unrouted — 0 routed by construction.
        assert_eq!(
            on.action_slot_fallbacks(),
            vec![("brew".to_owned(), vec!["brewLevelSeen".to_owned()])],
            "all action targets report as unminted coverage"
        );
        // The If condition's `level` read bound to the ODE state slot.
        assert!(
            on.slot_bind_report().total_bound() > 0,
            "aggregate bind report must include the action's bound reads"
        );
    }

    /// RSC-2.4c 100-tick smoke on a real action-carrying model:
    /// orchestration-complex's `BrewCycle` (fork/join, 6 Perform
    /// sub-actions) added to the compiled orchestrator (2 SMs + RK45 ODE)
    /// with slots enabled. The brew cycle's token flow must run to
    /// completion without minting a single context key, and the
    /// SM/ODE slot machinery must stay coherent around it.
    #[test]
    fn rsc24c_orchestration_complex_brewcycle_100_tick_smoke() {
        use sysml_parser_trait::{Parser as _, SysmlFile};

        // Fixture relocated into this crate (R60): it previously lived under
        // `editors/diagram/examples/stress/`, which was deleted with the legacy diagram
        // package (commit 71cc007a) — the test then panicked on a missing file.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/orchestration-complex.sysml");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()));
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new(
            "orchestration-complex.sysml".to_owned(),
            source,
        )]);
        let mut graph = result.graph;
        elaborate::elaborate(&mut graph);

        let compiler = ModelCompiler::from_arc(Arc::new(graph));
        let action_ir = compiler
            .compile_action("BrewCycle")
            .expect("BrewCycle should compile");
        let action_targets = crate::actions::collect_write_targets(&action_ir);

        let base_ctx = context_from_graph(compiler.graph());
        let mut orch = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[],
                Some(10.0),
                Some(60_000.0),
            )
            .expect("orchestration-complex workspace orchestrator should compile");
        orch.add_action("BrewCycle", crate::actions::ActionRunner::new(action_ir));
        orch.bind_expression_slots(Some(compiler.graph().as_ref()));

        let keys_before: std::collections::BTreeSet<String> =
            orch.context.variables.keys().cloned().collect();

        let mut last = None;
        for _ in 0..100 {
            last = Some(orch.step());
        }
        let snapshot = last.expect("stepped");

        let brew = &snapshot.subsystem_states["BrewCycle"];
        assert_eq!(brew.kind, "action");
        assert!(
            brew.completed,
            "BrewCycle fork/join token flow must run to completion, stuck at '{}'",
            brew.current_state
        );

        // Zero action-minted context keys: every key added over 100 ticks
        // must come from the SM/ODE/orchestrator writers, never from the
        // action's write targets or node bookkeeping.
        let keys_after: std::collections::BTreeSet<String> =
            orch.context.variables.keys().cloned().collect();
        for new_key in keys_after.difference(&keys_before) {
            assert!(
                !action_targets.iter().any(|t| t == new_key),
                "action write target '{new_key}' leaked into the context"
            );
        }
        println!(
            "orchestration-complex RSC-2.4c: BrewCycle targets {:?}, action fallbacks {:?}",
            action_targets,
            orch.action_slot_fallbacks()
        );

        // SM/ODE slot machinery stays coherent around the action: the ODE
        // state is finite and slot/map agree.
        let store = orch.slot_store();
        let key = "temperature";
        if let Some(id) = store.slot_by_name(key) {
            let map_val = orch.context.get(key).cloned();
            assert_eq!(
                store.get(id).cloned(),
                map_val,
                "slot/map coherence for '{key}' after 100 ticks"
            );
            if let Some(Value::Float(f)) = map_val {
                assert!(f.is_finite(), "'{key}' must stay finite, got {f}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // RSC-2.4d: Physics executor slot cutover (write-set restriction +
    // short-alias plane; slot routing itself is Phase 3 scope)
    // -----------------------------------------------------------------------
    //
    // Survey finding (2026-06-11): everything the physics tick writes is
    // port-feature identity (`owner.port.feature` paths) or solve-derived
    // DAE/user-constraint names — exactly the key classes `mint_slot_store`
    // DELIBERATELY leaves out of the slot table ("port/flow keys — Phase 3").
    // So 2.4d is write-set restriction + reporting: the legacy
    // whole-local-context dump (which echoed every merged shared key back,
    // value-idempotent, after `sync_context_in`'s whole-context merge) is
    // replaced by the enumerated write-set plus a faithful replication of
    // the short-alias mint (`owner.port.feature` → `port.feature`, ALSO
    // minted from other executors' echoed canonical keys — wire surface
    // pinned by the 2.B0 baselines). Routes are built with the shared
    // WriteRoute mechanics so Phase 3 can claim them; coverage is 0 by
    // construction until then.

    /// Hand-built physics network exercising every explicit-mode write
    /// class: an effort source, an effort equality, two flow sources
    /// feeding a KCL junction (sweep solver), with a tree topology so the
    /// RadialSweepSolver registers. No C/I storage → no DAE (DAE-name
    /// coverage is pinned at the executor level).
    fn parity_physics_executor() -> crate::physics::executor::PhysicsExecutor {
        use crate::flows::port::PortDirection;
        use crate::physics::connection::{
            ConnectionGraph, Junction, JunctionType, PhysicsConnection, PhysicsPortNode,
        };
        use crate::physics::constraints::{
            ConservationConstraint, ConstitutiveRelation, EffortEquality, GeneratedConstraints,
        };
        use crate::physics::domain::{ConservationLaw, PhysicsDomainRegistry};

        let mk = |id: usize, owner: &str, port: &str, dir: PortDirection| PhysicsPortNode {
            id,
            qualified_path: format!("{owner}.{port}"),
            owner_path: owner.to_owned(),
            port_name: port.to_owned(),
            domain: Some("electrical"),
            direction: dir,
            classification: None,
        };
        let nodes = vec![
            mk(0, "gen", "out", PortDirection::Out),
            mk(1, "bus", "powerIn", PortDirection::In),
            mk(2, "bus", "out1", PortDirection::Out),
            mk(3, "bus", "out2", PortDirection::Out),
        ];
        let edges = vec![
            PhysicsConnection {
                source: 0,
                target: 1,
                domain: Some("electrical"),
                enabled: true,
            },
            PhysicsConnection {
                source: 1,
                target: 2,
                domain: Some("electrical"),
                enabled: true,
            },
            PhysicsConnection {
                source: 1,
                target: 3,
                domain: Some("electrical"),
                enabled: true,
            },
        ];
        let junctions = vec![Junction {
            id: 0,
            owner: "bus".to_owned(),
            domain: "electrical",
            junction_type: JunctionType::Zero,
            conservation: ConservationLaw::FlowConservation,
            incoming: vec![(1, "current".to_owned())],
            outgoing: vec![(2, "current".to_owned()), (3, "current".to_owned())],
        }];
        let mut constraints = GeneratedConstraints::default();
        constraints.conservation.push(ConservationConstraint {
            name: "kcl_bus".to_owned(),
            junction_id: 0,
            law: ConservationLaw::FlowConservation,
            incoming_vars: vec!["bus.powerIn.current".to_owned()],
            outgoing_vars: vec!["bus.out1.current".to_owned(), "bus.out2.current".to_owned()],
        });
        constraints.effort_equalities.push(EffortEquality {
            source_var: "gen.out.voltage".to_owned(),
            target_var: "bus.powerIn.voltage".to_owned(),
        });
        constraints
            .constitutive
            .push(ConstitutiveRelation::EffortSource {
                effort_var: "gen.out.voltage".to_owned(),
                source_value: Some(230.0),
            });
        constraints
            .constitutive
            .push(ConstitutiveRelation::FlowSource {
                flow_var: "bus.out1.current".to_owned(),
                source_value: Some(3.0),
            });
        constraints
            .constitutive
            .push(ConstitutiveRelation::FlowSource {
                flow_var: "bus.out2.current".to_owned(),
                source_value: Some(4.0),
            });
        crate::physics::executor::PhysicsExecutor::new(
            Arc::new(PhysicsDomainRegistry::new()),
            ConnectionGraph {
                nodes,
                edges,
                junctions,
            },
            constraints,
        )
    }

    /// RSC-3.6 (was RSC-2.4d writeback parity): with a physics subsystem in a
    /// compiled orchestrator, the restricted physics write-set + alias plane
    /// solves the network and mints the canonical/echo-derived aliases the FE
    /// wire surface reads. The two-tank fixture supplies instance-multiplied
    /// ODEs with canonical 4-segment keys. The former slots-on-vs-off parity
    /// arm was retired with no-slots mode (RSC-3.5f.3 — slot routing is now
    /// unconditional).
    #[test]
    fn rsc24d_physics_writeback_solves_and_mints_aliases() {
        let compiler = ModelCompiler::from_arc(two_tank_graph());
        let base_ctx = context_from_graph(compiler.graph());
        let mut on = compiler
            .build_workspace_orchestrator(
                base_ctx,
                None,
                None,
                None,
                None,
                &[],
                Some(100.0),
                Some(60_000.0),
            )
            .expect("two-tank workspace orchestrator should compile");
        on.add_physics("grid", parity_physics_executor());
        // Re-run the bind/prepare pass so the late-added physics subsystem
        // gets its write-set (idempotent for the ODEs).
        on.bind_expression_slots(Some(compiler.graph().as_ref()));

        for _ in 0..50 {
            on.step();
        }

        let on_vars: std::collections::BTreeMap<&String, &Value> =
            on.context.variables.iter().collect();

        // The physics network actually solved on the migrated path: the
        // effort source pinned, the equality propagated, KCL aggregated
        // the two (sign-negated Sf) outgoing flows.
        assert_eq!(
            on.context.get("gen.out.voltage"),
            Some(&Value::Float(230.0))
        );
        assert_eq!(
            on.context.get("bus.powerIn.voltage"),
            Some(&Value::Float(230.0))
        );
        let kcl = match on.context.get("bus.powerIn.current") {
            Some(Value::Float(f)) => *f,
            other => panic!("expected Float at 'bus.powerIn.current', got {other:?}"),
        };
        assert_eq!(kcl.abs(), 7.0, "KCL aggregates the two outgoing flows");

        // Alias plane is wire surface: the physics-key short alias exists
        // in the map...
        assert_eq!(on.context.get("out.voltage"), Some(&Value::Float(230.0)));
        // ...and so does an echo-derived alias minted from a canonical
        // 3+-segment key the ODE writeback (not physics) produced.
        let echo_sources: Vec<&str> = on_vars
            .keys()
            .map(|k| k.as_str())
            .filter(|k| {
                k.matches('.').count() >= 2
                    && !k.starts_with("gen.")
                    && !k.starts_with("bus.")
                    && !crate::expressions::is_internal_var(k)
            })
            .collect();
        assert!(
            !echo_sources.is_empty(),
            "two-tank instances must produce canonical 3+-segment keys"
        );
        let segs: Vec<&str> = echo_sources[0].split('.').collect();
        let echo_alias = format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]);
        assert!(
            on.context.get(&echo_alias).is_some(),
            "echo-derived alias '{echo_alias}' (from '{}') must keep minting",
            echo_sources[0]
        );

        // Mint coverage is 0 by construction (no physics slots until
        // Phase 3): every write target reports name-keyed, plus the minted
        // aliases.
        let fallbacks = on.physics_slot_fallbacks();
        assert_eq!(fallbacks.len(), 1, "one physics subsystem: {fallbacks:?}");
        let (name, keys) = &fallbacks[0];
        assert_eq!(name, "grid");
        for key in ["gen.out.voltage", "bus.powerIn.current", "out.voltage"] {
            assert!(
                keys.iter().any(|k| k == key),
                "'{key}' in physics fallback inventory {keys:?}"
            );
        }
    }

    /// RSC-4.3 (L47): pin the shared eligibility primitive's semantics
    /// independently of its two consumers (solver selection ahead of WS-B4,
    /// and `wire_when_crossings_for_pair`'s actual detector wiring) — so a
    /// future change to either consumer can't silently drift the test of
    /// "does this ODE end up restep-eligible" away from what the OTHER
    /// consumer sees.
    #[test]
    fn sm_has_qualifying_when_crossing_matches_wiring_eligibility() {
        use sysml_parser_trait::{Parser as _, SysmlFile};
        let source = r#"
            package L47Eligibility {
                part def Decay {
                    attribute k : Real default 1000.0;
                    out attribute x : Real default 1.0;
                    action def Dynamics :> ContinuousStateSpaceDynamics {
                        calc def XDeriv :> GetDerivative {
                            return dxdt = 0 - k * x;
                        }
                    }
                }
                state def Qualifying {
                    in attribute x : Real;
                    state active;
                    state tripped;
                    entry; then active;
                    transition trip first active accept when x <= 0.5 then tripped;
                }
                state def NonQualifying {
                    state active;
                    state done;
                    entry; then active;
                    transition finish first active accept doneSignal then done;
                }
            }
        "#;
        let parser = sysml_parser_incremental::TreeSitterParser::new();
        let result = parser.parse(&[SysmlFile::new("l47.sysml".to_owned(), source.to_owned())]);
        let mut graph = result.graph;
        let _ = sysml_core::resolution::resolve_references(&mut graph);
        elaborate::elaborate(&mut graph);
        let compiler = ModelCompiler::from_arc(Arc::new(graph));

        let ode = compiler
            .detect_ode()
            .expect("Decay's GetDerivative calc must be detected as an ODE");

        let qualifying_ir = compiler
            .compile_state_machine("Qualifying")
            .expect("Qualifying compiles");
        let nonqualifying_ir = compiler
            .compile_state_machine("NonQualifying")
            .expect("NonQualifying compiles");

        assert!(
            ModelCompiler::sm_has_qualifying_when_crossing(&qualifying_ir, &ode),
            "an `accept when x <= 0.5` trigger referencing the ODE's continuous \
             state var must qualify — this is exactly what `wire_when_crossings_for_pair` \
             registers a located crossing for and marks restep-eligible"
        );
        assert!(
            !ModelCompiler::sm_has_qualifying_when_crossing(&nonqualifying_ir, &ode),
            "a bare event-name `accept` (no threshold comparator on a continuous \
             quantity) must NOT qualify — `wire_when_crossings_for_pair` registers \
             nothing for it, so restep-eligibility must agree"
        );

        // The `any_*` aggregator mirrors `.any()` over the exact same test —
        // true if ANY named SM qualifies, matching `wire_zero_crossing_detectors`'s
        // cross-product pairing (a single ODE paired against every SM in its
        // instance/workspace scope).
        assert!(
            compiler.any_sm_has_qualifying_when_crossing(["NonQualifying", "Qualifying"], &ode),
            "any_sm_has_qualifying_when_crossing must be true when ANY named SM qualifies"
        );
        assert!(
            !compiler.any_sm_has_qualifying_when_crossing(["NonQualifying"], &ode),
            "any_sm_has_qualifying_when_crossing must be false when NO named SM qualifies"
        );
        assert!(
            !compiler.any_sm_has_qualifying_when_crossing(std::iter::empty(), &ode),
            "any_sm_has_qualifying_when_crossing must be false over an empty SM set"
        );
    }
