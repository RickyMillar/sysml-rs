//! Definition processing — process_definition, standard_def_kind,
//! create_definition_rels, extract_definition. Definitions are typed
//! containers (`part def`, `attribute def`, `action def`, etc.).

use super::{AstBuilder, ModelGraphResult};
use sysml_core::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Value, VisibilityKind};
use sysml_parser_trait::extraction::DefinitionExtraction;
use sysml_parser_trait::relationship_builder::{
    create_conjugated_port_definition_with_key, create_conjugation_with_key,
    create_differencing_with_key, create_disjoining_with_key, create_feature_inverting_with_key,
    create_feature_typing_with_key, create_intersecting_with_key, create_subclassification_with_key,
    create_type_featuring_with_key, create_unioning_with_key,
};
use sysml_span::{Diagnostic, Span};
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    pub(super) fn process_definition(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        kind: ElementKind,
    ) -> Option<(ElementId, CanonicalKey)> {
        let mut extraction = self.extract_definition(node);
        let span = self.node_span(node);

        let (parent_key_resolved, child_key, sibling_index) =
            self.prep_canonical_key(parent_key, parent_id, &kind, extraction.name.as_deref());
        // Hold onto the *port def's* parent key (not its own child_key) so the
        // implicit ConjugatedPortDefinition created below threads off the same
        // namespace key as its sibling.
        let conjugated_parent_key = parent_key_resolved.clone();
        extraction.parent_key = Some(parent_key_resolved);
        extraction.sibling_index = sibling_index;

        let mut element = extraction.build_element(kind.clone(), Some(span.clone()));
        if let Some(name_node) = self.find_name_node(node) {
            element.name_span = Some(self.node_span(&name_node));
        }

        // §8.3.8.2 validateEnumerationDefinitionIsVariation: an
        // EnumerationDefinition is semantically a variation whose allowed
        // variants are its enumeratedValues, so `isVariation` is always true
        // (this also satisfies S050 for the VariantMembership that wraps each
        // enumerated value — the owning namespace of a VariantMembership must
        // be a variation). `isVariation` implies `isAbstract` (spec §14476, and
        // "a variation is always abstract" §3969), so stamp both — otherwise
        // S080 ("a variation must be abstract") fires spuriously on every valid
        // `enum def`. The model carries the implication rather than leaving the
        // derivation to a validation-time inference.
        if kind == ElementKind::EnumerationDefinition {
            element.set_prop("isVariation", Value::Bool(true));
            element.set_prop("isAbstract", Value::Bool(true));
        }

        // Extract result expression for constraint/calc definitions
        // TS-1.4: also capture the CST node so we can walk it through
        // `ExpressionBuilder` after the parent element is added to the graph.
        let mut pending_result_expr: Option<(Span, String)> = None;
        let mut pending_result_expr_node: Option<Node<'a>> = None;
        let is_constraint_or_calc_def = matches!(
            kind,
            ElementKind::ConstraintDefinition | ElementKind::CalculationDefinition
        );
        if is_constraint_or_calc_def {
            let body = self
                .find_child_node(node, "constraint_body")
                .or_else(|| self.find_child_node(node, "function_body"))
                // G04b: calc_def uses calc_body (a function_body peer admitting anon params)
                .or_else(|| self.find_child_node(node, "calc_body"));
            if let Some(body) = body {
                if let Some(result_expr) = self.find_child_node(&body, "result_expression") {
                    let expr_span = self.node_span(&result_expr);
                    let expr_text = self.node_text(&result_expr).trim().to_owned();
                    let prop_name = if kind == ElementKind::CalculationDefinition {
                        "expr"
                    } else {
                        "constraint"
                    };
                    element.set_prop(prop_name, Value::String(expr_text.clone()));
                    pending_result_expr = Some((expr_span, expr_text));
                    pending_result_expr_node = Some(result_expr);
                }
            }
        }

        let def_name = element.name.clone();
        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            Some(&conjugated_parent_key),
            &child_key,
            &mut result.graph,
        );

        // Create ResultExpressionMembership child for the result expression
        if let Some((expr_span, expr_text)) = pending_result_expr {
            let owner_id = Some(id.clone());
            let (_, mut rem) = self.mint_direct_element(
                Some(&child_key),
                &owner_id,
                ElementKind::ResultExpressionMembership,
                None,
            );
            rem.spans.push(expr_span);
            rem.set_prop("expression", Value::String(expr_text));
            rem.owner = Some(id.clone());
            result.graph.add_element(rem);
        }

        // TS-1.4 gap #4: emit structured expression subtree for the result
        // expression. Pest emits these for constraint defs already; calc defs
        // were string-only on both sides until now.
        if let Some(result_expr) = pending_result_expr_node {
            if let Some(inner) = self.find_first_named_child(&result_expr) {
                let builder = crate::expression_elements::ExpressionBuilder::new(
                    self.source,
                    self.file_path,
                );
                builder.process_with_key(inner, id.clone(), &child_key, &mut result.graph);
            }
        }

        // Create Specialization elements with precise per-type_ref spans
        self.create_definition_rels(node, &id, Some(&child_key), &mut result.graph);

        // Lower type-relationship clauses (unions/intersects/differences/
        // disjoint) that the grammar parses but the base lowering ignored.
        self.create_type_relationship_rels(node, &id, Some(&child_key), &mut result.graph);

        // Per SysML spec, every PortDefinition implicitly owns a ConjugatedPortDefinition.
        // It is owned by the same parent namespace as the PortDefinition, so its
        // canonical key threads off `parent_key_resolved` (the port def's own
        // parent) — not `child_key` (the port def itself).
        if kind == ElementKind::PortDefinition {
            if let Some(name) = def_name {
                create_conjugated_port_definition_with_key(
                    &mut result.graph,
                    id.clone(),
                    &name,
                    parent_id.clone(),
                    Some(span),
                    &conjugated_parent_key,
                );
            }
        }

        Some((id, child_key))
    }

    /// Lower an `enum_member` (CST) inside an `enum def` body to a distinct
    /// `EnumerationUsage` (SysML `EnumeratedValue`, §8.3.8.3).
    ///
    /// Grammar: `EnumerationUsageMember returns SysML::VariantMembership :
    /// MemberPrefix ownedRelatedElement += EnumeratedValue` and
    /// `EnumeratedValue returns SysML::EnumerationUsage : ... Usage`
    /// (SysML.xtext rules `EnumerationUsageMember` / `EnumeratedValue`).
    ///
    /// Each enumerated value is:
    /// - a distinct `EnumerationUsage` element (closing the
    ///   `enumeration-usage-not-distinct` gap, where these produced zero
    ///   elements),
    /// - owned by its `EnumerationDefinition` through a `VariantMembership`
    ///   (the metaclass the grammar's member rule returns), so it registers
    ///   in the enumeration's scope and a qualified reference (`Color::red`)
    ///   resolves,
    /// - typed by the owning `EnumerationDefinition` (§8.3.8.3: an
    ///   EnumerationUsage's `/enumerationDefinition` redefines
    ///   `attributeDefinition`).
    pub(super) fn process_enum_member(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        // An enumerated value is only well-formed as a member of an
        // EnumerationDefinition. If we reach here without an owning enum def
        // (structurally impossible via the grammar), drop rather than mint an
        // orphan variant (Principle #1: no soft fallback).
        let enum_def_id = parent_id.clone()?;

        let span = self.node_span(node);
        let name = self
            .find_field_text(node, "name")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        // An enumerated value is either a named Usage (`enum red;`) or the
        // unnamed value form (`= 60.0;`, SysML.xtext EnumeratedValue → Usage,
        // used by e.g. `enum def SizeChoice`). A member with NEITHER a name nor
        // a value is malformed — surface a diagnostic and skip rather than mint
        // an unreferenceable empty variant (Principle #1).
        let has_value = self.find_child_node(node, "default_value").is_some();
        if name.is_none() && !has_value {
            result.diagnostics.push(
                Diagnostic::error("enumerated value must have a name or a value".to_owned())
                    .with_span(span),
            );
            return None;
        }

        let (owner_key, child_key, _sibling_index) = self.prep_canonical_key(
            parent_key,
            parent_id,
            &ElementKind::EnumerationUsage,
            name.as_deref(),
        );

        let mut element = Element::new_with_key(ElementKind::EnumerationUsage, &child_key);
        element.name = name;
        element.spans.push(span.clone());
        if let Some(name_node) = node.child_by_field_name("name") {
            element.name_span = Some(self.node_span(&name_node));
        }

        // Wrap the enumerated value in a VariantMembership owned by the
        // enumeration (SysML.xtext `EnumerationUsageMember`). The enum def was
        // stamped `isVariation = true` in `process_definition`, satisfying S050.
        let member_id = result.graph.add_owned_element_with_membership_kind_key(
            element,
            enum_def_id.clone(),
            ElementKind::VariantMembership,
            VisibilityKind::Public,
            &owner_key,
            &child_key,
        );

        // §8.3.8.3: the enumerated value is typed by its owning enumeration.
        // The type is structural (the owner), not a textual reference the
        // author wrote, so resolve it directly to the enum def id while also
        // recording the name so the resolver/consumers see a normal typing.
        if let Some(type_name) = result
            .graph
            .get_element(&enum_def_id)
            .and_then(|e| e.name.clone())
        {
            let typing_id = create_feature_typing_with_key(
                &mut result.graph,
                member_id.clone(),
                type_name,
                Some(span),
                &child_key,
                "typing",
                0,
            );
            if let Some(typing) = result.graph.get_element_mut(&typing_id) {
                typing.set_prop("type", Value::Ref(enum_def_id));
            }
        }

        // Lower a `= <expr>` fixed value — both the unnamed value form and a
        // named member with a value — to an AST subtree, the same path usages
        // use (`emit_default_value_expression`).
        self.emit_default_value_expression(node, &member_id, &child_key, &mut result.graph);

        Some((member_id, child_key))
    }


    /// Determine the ElementKind for a `kerml_definition` node by scanning anonymous children.
    ///
    /// The tree-sitter grammar covers the KerML keyword set: `classifier`, `class`,
    /// `struct`, `datatype`, `function`, `behavior`, `interaction`, `metaclass`,
    /// `assoc`, `type`, `predicate`, with an optional `assoc`/`individual` prefix.
    /// Per the KerML spec (KerML.xtext lines 784-1060), each keyword maps to a
    /// specific element kind. The `assoc struct` two-token combination maps to
    /// `AssociationStructure`.
    pub(super) fn kerml_definition_kind(&self, node: &Node<'a>) -> ElementKind {
        let mut cursor = node.walk();
        let mut saw_assoc = false;
        for child in node.children(&mut cursor) {
            if child.is_named() {
                // Stop at `_name` (the definition body has been consumed)
                continue;
            }
            let text = self.node_text(&child);
            match text {
                // The optional `assoc` prefix; `assoc struct` → AssociationStructure
                "assoc" => {
                    // If this is followed by `struct`, the combination is
                    // AssociationStructure; otherwise it's a plain Association.
                    saw_assoc = true;
                }
                "individual" => {
                    // `individual` is a prefix modifier; doesn't change the kind
                    // but doesn't terminate scanning either.
                }
                "classifier" => return ElementKind::Classifier,
                "class" => return ElementKind::Class,
                "struct" => {
                    if saw_assoc {
                        return ElementKind::AssociationStructure;
                    }
                    return ElementKind::Structure;
                }
                "datatype" => return ElementKind::DataType,
                "function" => return ElementKind::Function,
                "behavior" => return ElementKind::Behavior,
                "interaction" => return ElementKind::Interaction,
                "metaclass" => return ElementKind::Metaclass,
                "type" => return ElementKind::Type,
                "predicate" => return ElementKind::Predicate,
                _ => {}
            }
        }
        // Bare `assoc <Name>` with no further keyword is an Association.
        if saw_assoc {
            return ElementKind::Association;
        }
        // Grammar (`helpers/patterns.js:KERML_DEF_KEYWORDS`) is a fixed closed set,
        // so this default is unreachable in practice; left as a safety net.
        ElementKind::Definition
    }

    /// Determine the ElementKind for a `standard_def` node by scanning anonymous children.
    ///
    /// The tree-sitter grammar merges 9 definition types (part, attribute, port,
    /// connection, interface, item, allocation, occurrence, flow) into a single
    /// `standard_def` rule. No `field()` wrapper on the keyword to avoid LR state
    /// explosion, so we scan anonymous children for the keyword text before "def".
    pub(super) fn standard_def_kind(&self, node: &Node<'a>) -> ElementKind {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // Stop at "def" — the keyword always precedes it
            let text = self.node_text(&child);
            if text == "def" {
                break;
            }
            // Skip named children (visibility_indicator, etc.) and non-keyword tokens
            if child.is_named() {
                continue;
            }
            match text {
                "part" => return ElementKind::PartDefinition,
                "attribute" => return ElementKind::AttributeDefinition,
                "port" => return ElementKind::PortDefinition,
                "connection" => return ElementKind::ConnectionDefinition,
                "interface" => return ElementKind::InterfaceDefinition,
                "item" => return ElementKind::ItemDefinition,
                "allocation" => return ElementKind::AllocationDefinition,
                "occurrence" => return ElementKind::OccurrenceDefinition,
                "flow" => return ElementKind::FlowDefinition,
                _ => {}
            }
        }
        ElementKind::Definition
    }


    /// Create Subclassification elements from the supertype_list child,
    /// using each type_ref's span instead of the whole definition span.
    /// Per KerML spec, `:>` on a Classifier creates Subclassification (not generic Specialization).
    pub(super) fn create_definition_rels(
        &self,
        node: &Node<'a>,
        def_id: &ElementId,
        def_key: Option<&CanonicalKey>,
        graph: &mut ModelGraph,
    ) {
        let owner_key = self.resolve_parent_key(def_key);
        if let Some(supertype_list) = self.find_child_node(node, "supertype_list") {
            let child_count = supertype_list.child_count();
            let mut sib_idx: usize = 0;
            for i in 0..child_count {
                if let Some(child) = supertype_list.child(i) {
                    // Accept both wrapped (`type_ref → feature_chain`) and
                    // direct (`feature_chain` / `qualified_name`) children so
                    // every spec-faithful `:>` form on a definition reaches
                    // `create_subclassification_with_key` (TS-1.3 / gap #7).
                    let qname_opt = match child.kind() {
                        "type_ref" => self.extract_type_ref_from_node(&child),
                        "feature_chain" | "qualified_name" => {
                            Some(self.node_text(&child).to_owned())
                        }
                        _ => None,
                    };
                    if let Some(qname) = qname_opt {
                        create_subclassification_with_key(
                            graph,
                            def_id.clone(),
                            qname,
                            Some(self.node_span(&child)),
                            &owner_key,
                            "subclassification",
                            sib_idx,
                        );
                        sib_idx += 1;
                    }
                }
            }
        }
    }


    /// Lower KerML type-relationship clauses (§7.3) that the grammar parses but
    /// the base definition/usage lowering ignored: `unions` / `intersects` /
    /// `differences` (bare `feature_chain` targets), `disjoint from`
    /// (`type_ref` targets), `featured by` → TypeFeaturing, and `inverse of` →
    /// FeatureInverting (both single `feature_chain` targets). Each mints a real
    /// relationship element owned by the declaring type/feature (the vocab
    /// source role, e.g. `typeUnioned`), with the target captured as
    /// `unresolved_<targetRole>` for name resolution. Conjugation
    /// (`~`/`conjugates`) is a separate grammar gap and is NOT handled here.
    pub(super) fn create_type_relationship_rels(
        &self,
        node: &Node<'a>,
        owner_id: &ElementId,
        owner_key: Option<&CanonicalKey>,
        graph: &mut ModelGraph,
    ) {
        let parent_key = self.resolve_parent_key(owner_key);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "unions_clause" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_unioning_with_key,
                ),
                "intersects_clause" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_intersecting_with_key,
                ),
                "differences_clause" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_differencing_with_key,
                ),
                "disjoint_clause" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_disjoining_with_key,
                ),
                "featuring" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_type_featuring_with_key,
                ),
                "inverting" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_feature_inverting_with_key,
                ),
                // ConjugationPart `~`/`conjugates`: the declaring type is the
                // implicit conjugatedType (source); the target is originalType.
                "conjugation_clause" => self.mint_type_relationship_targets(
                    &child,
                    owner_id,
                    &parent_key,
                    graph,
                    create_conjugation_with_key,
                ),
                _ => {}
            }
        }
    }

    /// Mint one relationship per target reference inside a type-relationship
    /// clause. Each target is a direct child of the clause — the grammar is
    /// inconsistent about the wrapper (`unions`/`differences` emit a bare
    /// `feature_chain`, `intersects`/`disjoint from` wrap it in a `type_ref`),
    /// so accept either at the direct-child level. `builder` is the per-kind
    /// relationship constructor; all six share the identical signature.
    /// Per-target `sibling_index` keeps the minted relationships' canonical
    /// keys distinct and source-ordered.
    fn mint_type_relationship_targets(
        &self,
        clause: &Node<'a>,
        owner_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
        builder: fn(&mut ModelGraph, ElementId, String, Option<Span>, &CanonicalKey, usize) -> ElementId,
    ) {
        let mut idx = 0usize;
        let mut cursor = clause.walk();
        for target in clause.children(&mut cursor) {
            let qname = match target.kind() {
                "type_ref" => self.extract_type_ref_from_node(&target),
                "feature_chain" | "qualified_name" => {
                    Some(self.node_text(&target).trim().to_owned())
                }
                _ => None,
            }
            .filter(|q| !q.is_empty());
            if let Some(q) = qname {
                builder(
                    graph,
                    owner_id.clone(),
                    q,
                    Some(self.node_span(&target)),
                    parent_key,
                    idx,
                );
                idx += 1;
            }
        }
    }

    /// Extract definition information from a node.
    pub(super) fn extract_definition(&self, node: &Node<'a>) -> DefinitionExtraction {
        let mut extraction = DefinitionExtraction {
            name: self.find_name_field(node),
            short_name: self.find_short_name(node),
            ..Default::default()
        };

        // Check for abstract/variation in definition_prefix (custom-body defs)
        if let Some(prefix_text) = self.find_child_text(node, "definition_prefix") {
            extraction.is_abstract = prefix_text.contains("abstract");
            extraction.is_variation = prefix_text.contains("variation");
        }

        // Also check for "abstract" as a direct anonymous child (standard_def has
        // optional("abstract") inline rather than in a definition_prefix node)
        if !extraction.is_abstract && self.has_anonymous_child(node, "abstract") {
            extraction.is_abstract = true;
        }

        // Extract supertype list (specializations)
        extraction.subclassifications = self.extract_supertype_list(node);

        extraction
    }

}
