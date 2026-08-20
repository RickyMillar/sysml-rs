//! Central dispatch — `build` drives the iterative CST walk, `process_node`
//! routes each tree-sitter node kind to the appropriate processor in a
//! sibling module (packages, definitions, usages, requirements, states,
//! imports, …). Reparse-stable canonical keys cascade through the work
//! stack so the per-element-kind processors can mint deterministic IDs
//! per ADR-009.

use super::node_helpers::{describe_context, find_enclosing_name};
use super::{AstBuilder, ModelGraphResult, WorkItem, MAX_DIAGNOSTICS_PER_ERROR_NODE};
use sysml_core::{CanonicalKey, ElementId, ElementKind, Value};
use sysml_parser_trait::relationship_builder::create_feature_typing_with_key;
use sysml_span::{Diagnostic, Span};
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    /// Build the ModelGraph from the root node.
    #[allow(clippy::indexing_slicing)]
    pub(super) fn build(&mut self, root: Node<'a>, result: &mut ModelGraphResult) {
        // Use iterative traversal with a work stack to avoid stack overflow
        let mut work_stack: Vec<WorkItem<'a>> = vec![WorkItem {
            node: root,
            parent_id: None,
            parent_key: None,
        }];

        // Track the most recently created element for each parent scope.
        //
        // The tree-sitter grammar sometimes splits a single SysML declaration
        // into two sibling CST nodes. For example:
        //
        //   part def Foo :> Bar { body }
        //   → part_def "part def Foo" + feature_redefinition ":> Bar { body }"
        //
        //   part x : Foo { body }
        //   → standard_usage "part" + feature_declaration "x : Foo { body }"
        //
        // When we encounter a feature_declaration or feature_redefinition, we
        // look up the element created by the preceding sibling (tracked here)
        // and associate the declaration's children with that element.
        let mut last_sibling_element: std::collections::HashMap<Option<ElementId>, ElementId> =
            std::collections::HashMap::new();
        // Parallel to `last_sibling_element`: tracks the canonical key of the
        // most recently created element under each parent scope, so a
        // following `feature_declaration` / `feature_redefinition` split
        // sibling can re-use the parent key when augmenting that element's
        // descendants.
        let mut last_sibling_key: std::collections::HashMap<Option<ElementId>, CanonicalKey> =
            std::collections::HashMap::new();

        while let Some(work) = work_stack.pop() {
            let node = work.node;

            // Skip nodes consumed by a preceding sibling (e.g., action_usage absorbed by entry_action)
            if self.consumed_nodes.contains(&node.id()) {
                continue;
            }

            // Handle ERROR nodes
            if node.is_error() {
                let span = self.node_span(&node);
                let error_text = &self.source[span.start..span.end.min(self.source.len())];

                // Build context from parent node, including enclosing name if available.
                // e.g. "in definition body of `sysml_text`" instead of bare "in definition body".
                let context: Option<String> = node.parent().and_then(|p| {
                    let base = describe_context(p.kind())?;
                    // Walk up to find the nearest named ancestor for richer context
                    let owner_name = find_enclosing_name(p, self.source);
                    match owner_name {
                        Some(name) => Some(format!("{} of `{}`", base, name)),
                        None => Some(base.to_owned()),
                    }
                });

                // Previous sibling note (computed once, shared across split diagnostics)
                let prev_note = node.prev_sibling().and_then(|prev| {
                    if prev.is_error() {
                        return None;
                    }
                    let prev_text: String = self.node_text(&prev).chars().take(20).collect();
                    if prev_text.trim().is_empty() {
                        Some(format!("after `{}`", prev.kind()))
                    } else {
                        Some(format!("after `{}`", prev_text.trim()))
                    }
                });

                // Split large multi-line ERROR nodes into per-line diagnostics
                // so each bad line gets its own inline error in the editor.
                let start_row = node.start_position().row;
                let end_row = node.end_position().row;
                let is_multiline = end_row > start_row;

                if is_multiline {
                    // Emit per-line diagnostics, capped at MAX_DIAGNOSTICS_PER_ERROR_NODE.
                    // Beyond the cap, emit a single summary so the user knows there
                    // are more errors but isn't flooded with cascade noise.
                    let mut line_start = span.start;
                    let mut emitted = 0usize;
                    let mut total_non_empty = 0usize;
                    let mut first_span_start = None;
                    for line in error_text.split('\n') {
                        let trimmed = line.trim();
                        let line_end = line_start + line.len();
                        if !trimmed.is_empty() {
                            total_non_empty += 1;
                            // Find the trimmed content offset within the line
                            let trim_offset = line.len() - line.trim_start().len();
                            let trim_start = line_start + trim_offset;
                            let trim_end =
                                line_start + line.len() - (line.len() - line.trim_end().len());

                            if first_span_start.is_none() {
                                first_span_start = Some(trim_start);
                            }

                            if emitted < MAX_DIAGNOSTICS_PER_ERROR_NODE {
                                let preview: String = trimmed.chars().take(40).collect();
                                let msg = self.format_error_message(&preview, context.as_ref());

                                let mut diag = Diagnostic::error(msg).with_span(Span::new(
                                    self.file_path,
                                    trim_start,
                                    trim_end,
                                ));

                                // Only add sibling note to the first line
                                if emitted == 0 {
                                    if let Some(ref note) = prev_note {
                                        diag = diag.with_note(note.clone());
                                    }
                                }

                                result.diagnostics.push(diag);
                                emitted += 1;
                            }
                        }
                        line_start = line_end + 1; // +1 for the '\n'
                    }

                    // Summary diagnostic for suppressed cascade lines
                    let suppressed = total_non_empty.saturating_sub(emitted);
                    if suppressed > 0 {
                        let msg = format!(
                            "{} more error{} in this region (fix the error above first)",
                            suppressed,
                            if suppressed == 1 { "" } else { "s" },
                        );
                        let summary_start = first_span_start.unwrap_or(span.start);
                        result
                            .diagnostics
                            .push(Diagnostic::info(msg).with_span(Span::new(
                                self.file_path,
                                summary_start,
                                span.end,
                            )));
                    }
                } else {
                    // Single-line ERROR: emit one diagnostic
                    let preview: String = error_text.trim().chars().take(40).collect();
                    let msg = self.format_error_message(&preview, context.as_ref());

                    let mut diag = Diagnostic::error(msg).with_span(span);

                    if let Some(note) = prev_note {
                        diag = diag.with_note(note);
                    }

                    result.diagnostics.push(diag);
                }

                // Skip children of error nodes, but continue with siblings
                continue;
            }

            // Handle MISSING nodes (expected token not found)
            if node.is_missing() {
                let expected = node.kind();

                let context = node.parent().and_then(|p| describe_context(p.kind()));

                // Use end of previous sibling for the span so the error highlights
                // where the missing token should be inserted, not where the parser
                // encountered the next (unexpected) token. This also makes code
                // actions like "insert missing semicolon" insert at the right place.
                let (span, prev_name) = match node.prev_sibling() {
                    Some(prev) if !prev.is_error() && !prev.is_missing() => {
                        let end = prev.end_byte();
                        let start = end.saturating_sub(1).max(prev.start_byte());
                        let text = self.node_text(&prev);
                        let trimmed = text.trim();
                        let name = if !trimmed.is_empty() && trimmed.len() <= 30 {
                            Some(trimmed.to_owned())
                        } else {
                            None
                        };
                        (Span::new(self.file_path, start, end), name)
                    }
                    _ => (self.node_span(&node), None),
                };

                // "found X" — show what the parser hit instead of the expected token
                let found = node.next_sibling().and_then(|next| {
                    if next.is_error() || next.is_missing() {
                        return None;
                    }
                    let text: String = self.node_text(&next).trim().chars().take(20).collect();
                    if text.is_empty() {
                        None
                    } else {
                        Some(text)
                    }
                });

                let mut msg = match (&context, &prev_name) {
                    (Some(ctx), Some(name)) => {
                        format!("expected `{}` after '{}' {}", expected, name, ctx)
                    }
                    (Some(ctx), None) => format!("expected `{}` {}", expected, ctx),
                    (None, Some(name)) => {
                        format!("expected `{}` after '{}'", expected, name)
                    }
                    (None, None) => format!("expected `{}`", expected),
                };
                if let Some(found) = found {
                    msg.push_str(&format!(", found `{}`", found));
                }

                result
                    .diagnostics
                    .push(Diagnostic::error(msg).with_span(span));
                continue;
            }

            // Handle feature_declaration / feature_redefinition split siblings.
            // These nodes contain the name, typing, supertype list, and body
            // of a definition/usage whose keyword was parsed as a separate
            // preceding sibling node.
            let is_split_decl =
                matches!(node.kind(), "feature_declaration" | "feature_redefinition");

            let processed = if is_split_decl {
                // Only augment if the preceding sibling element is a definition,
                // usage, or control node (the split pattern applies to
                // `part_def Foo :> Bar`, `fork forkPrep;`, etc.).
                // Import elements must NOT be augmented — they have their own
                // narrow spans and shouldn't absorb feature_declaration siblings.
                //
                // Control nodes (fork, join, merge, decide) need this because
                // an sl_note comment before a named control_flow_node can cause
                // tree-sitter's GLR resolver to split the name into a separate
                // feature_declaration sibling (e.g. `fork` + `forkPrep;`).
                //
                // TS-1.4 gap #4: feature_declaration members inside a
                // `@DataSource { ... }` body must NOT augment a previous
                // sibling — each `name = expr` line is a standalone
                // ReferenceUsage. Detect by checking whether the *parent* of
                // these siblings is a MetadataUsage / metadata-bearing node.
                let parent_is_metadata = work
                    .parent_id
                    .as_ref()
                    .and_then(|p| result.graph.elements.get(p))
                    .map(|e| {
                        e.kind == ElementKind::MetadataUsage
                            || e.kind == ElementKind::MetadataDefinition
                    })
                    .unwrap_or(false);
                // B1: a feature_declaration that carries its OWN usage_prefix
                // is a complete standalone declaration, not a split
                // continuation — in the split pattern the prefix lives in the
                // preceding keyword node. Repro: two `end ref X references Y;`
                // ends in one connection body — GLR parses the first as
                // standard_usage, the second as feature_declaration; fusing
                // the second onto the first collapsed both ends into one
                // element (its ReferenceSubsetting re-parented onto end 1).
                let has_own_prefix = node.kind() == "feature_declaration"
                    && self.find_child_node(&node, "usage_prefix").is_some();
                let prev_is_augmentable = !parent_is_metadata
                    && !has_own_prefix
                    && last_sibling_element
                        .get(&work.parent_id)
                        .and_then(|eid| result.graph.elements.get(eid))
                        .map(|e| {
                            e.kind.is_definition() || e.kind.is_usage() || e.kind.is_control_node()
                        })
                        .unwrap_or(false);
                if prev_is_augmentable {
                    let prev_eid = last_sibling_element[&work.parent_id].clone();
                    // Augment doesn't mint a new element, so its canonical
                    // key (for descendant scoping) is the previously-minted
                    // sibling's key. If we never recorded it (e.g. the prev
                    // element was minted by a code path that didn't return a
                    // key), fall back to the work-item's parent key.
                    let prev_key = last_sibling_key
                        .get(&work.parent_id)
                        .cloned()
                        .or_else(|| work.parent_key.clone());
                    self.augment_from_split_sibling(&node, &prev_eid, prev_key.as_ref(), result);
                    prev_key.map(|k| (prev_eid, k))
                } else {
                    // No suitable preceding element — process normally as fallback
                    self.process_node(&node, &work.parent_id, work.parent_key.as_ref(), result)
                }
            } else {
                self.process_node(&node, &work.parent_id, work.parent_key.as_ref(), result)
            };

            let (element_id, child_canonical_key) = match processed {
                Some((eid, key)) => (Some(eid), Some(key)),
                None => (None, None),
            };

            // Track last element for this parent scope (only for element-creating
            // nodes, not for split declarations which augment an existing element)
            if let Some(ref eid) = element_id {
                if !is_split_decl {
                    last_sibling_element.insert(work.parent_id.clone(), eid.clone());
                    if let Some(ref key) = child_canonical_key {
                        last_sibling_key.insert(work.parent_id.clone(), key.clone());
                    }
                }
            }

            // Determine the parent for children
            let child_parent = element_id.or(work.parent_id.clone());
            // Determine the canonical-key parent for children. When this node
            // produced an element, its own canonical key becomes the parent
            // key for descendants. Otherwise we pass our own parent key down
            // (structural nodes like `package_body` have no element; their
            // children resolve against the enclosing element's key).
            let child_parent_key = match child_canonical_key {
                Some(key) => Some(key),
                None => work.parent_key.clone(),
            };

            // Add children to work stack in reverse order (so they're processed left-to-right)
            let child_count = node.child_count();
            for i in (0..child_count).rev() {
                if let Some(child) = node.child(i) {
                    work_stack.push(WorkItem {
                        node: child,
                        parent_id: child_parent.clone(),
                        parent_key: child_parent_key.clone(),
                    });
                }
            }
        }
    }

    /// Process a single node and return its ElementId — together with the
    /// canonical key used to mint it — when one was created. The returned
    /// canonical key is propagated as the parent key for the node's
    /// descendants so that ADR-009 reparse-stable IDs cascade.
    #[allow(clippy::indexing_slicing)]
    pub(super) fn process_node(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let kind = node.kind();

        match kind {
            // Package declarations
            "package_decl" => self.process_package(node, parent_id, parent_key, result, false),
            "library_package" => self.process_package(node, parent_id, parent_key, result, true),

            // Merged standard definitions (part, attribute, port, connection,
            // interface, item, allocation, occurrence, flow)
            "standard_def" => {
                let kind = self.standard_def_kind(node);
                self.process_definition(node, parent_id, parent_key, result, kind)
            }
            "annotated_connection_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConnectionDefinition,
            ),

            // G04b: `calc def` peeled into its own rule (calc_body admits anonymous
            // typed params). Same lowering as the old generic `definition`/"calc" path.
            "calc_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::CalculationDefinition,
            ),

            // Custom-body definitions (keep individual rules)
            "action_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ActionDefinition,
            ),
            "state_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::StateDefinition,
            ),
            "requirement_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::RequirementDefinition,
            ),
            "constraint_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConstraintDefinition,
            ),
            "enum_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::EnumerationDefinition,
            ),
            // Enumerated values inside an `enum def` body. Without this arm the
            // members hit the `_ => None` default and were dropped entirely
            // (gap `enumeration-usage-not-distinct`); each now lowers to a
            // distinct EnumerationUsage. `enum_body` itself is a structural
            // container (handled in the skip list below); its `enum_member`
            // children are pushed with the enum def as their parent.
            "enum_member" => self.process_enum_member(node, parent_id, parent_key, result),

            // Standalone EnumerationUsage `enum e : Color;` (SysML.xtext
            // EnumerationUsage:785-788 `UsagePrefix EnumerationUsageKeyword
            // Usage` — a REAL metaclass, SysML-vocab.ttl:343). An ORDINARY
            // member of its owner: VariantMembership wrapping applies only to
            // enumerated values inside an `enum def` body (EnumeratedValue,
            // handled by `enum_member` above / ad4c95ba). Typing (`: Color`),
            // subsetting (`:> …`) and the usage_prefix flags (`variation` →
            // isVariation, …) all ride the standard process_usage path.
            "enum_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::EnumerationUsage,
            ),

            // `individual def X;` (SysML.xtext IndividualDefinition:813-817).
            // There is NO IndividualDefinition metaclass — the rule RETURNS
            // SysML::OccurrenceDefinition with `isIndividual ?= 'individual'`
            // (SysML-vocab.ttl:718-724 OccurrenceDefinition, :1718
            // isIndividual). The spec's EmptyMultiplicityMember (xtext
            // :819-825 — an owned empty Multiplicity that, with isIndividual,
            // caps the definition at a single instance) is CONSCIOUSLY
            // simplified flags-first: `isIndividual` carries the constraint;
            // no phantom Multiplicity child is minted (mirrors the usage-side
            // occurrence-prefix lowering, 0ad56827). The `abstract` prefix
            // composes via extract_definition's anonymous-child check.
            "individual_def" => {
                let (id, ck) = self.process_definition(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::OccurrenceDefinition,
                )?;
                if let Some(elem) = result.graph.get_element_mut(&id) {
                    elem.set_prop("isIndividual", true);
                }
                Some((id, ck))
            }

            // Merged standard usages (part, attribute, item, occurrence, ref)
            "standard_usage" => {
                let kind = self.standard_usage_kind(node);
                self.process_usage(node, parent_id, parent_key, result, kind)
            }
            // Non-standard usage types
            "action_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ActionUsage,
            ),
            "state_usage" => {
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::StateUsage,
                )?;
                // Detect "parallel" anonymous keyword token
                if self.has_anonymous_child(node, "parallel") {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("isParallel", true);
                    }
                }
                Some((id, _ck))
            }
            // Exhibit state usage (G23) — peeled from the generic `usage` fallback
            // into a dedicated rule (usages.js) so `exhibit state <name> : <Type>;`
            // mints ONE ExhibitStateUsage instead of an empty usage + a phantom
            // sibling StateUsage. Mirrors state_usage's dispatch exactly: the CST
            // shape (name/typing/state_body/parallel) is identical, so no new
            // extraction logic is needed here — the fix is entirely grammar-side.
            "exhibit_state_usage" => {
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::ExhibitStateUsage,
                )?;
                if self.has_anonymous_child(node, "parallel") {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("isParallel", true);
                    }
                }
                Some((id, _ck))
            }
            "port_usage" => {
                self.process_usage(node, parent_id, parent_key, result, ElementKind::PortUsage)
            }
            "connection_usage" | "annotated_connection_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConnectionUsage,
            ),
            "connection_end_usage" => {
                let (id, key) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::ReferenceUsage,
                )?;
                if let Some(end) = result.graph.get_element_mut(&id) {
                    end.set_prop("isEnd", true);
                    end.set_prop("isReference", true);
                }
                Some((id, key))
            }
            "interface_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::InterfaceUsage,
            ),
            "allocation_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::AllocationUsage,
            ),
            // B1: `dependency [X from] a to b { @Refinement; }` → Dependency
            // element with unresolved client/supplier props (was a silent
            // drop to `_ => None`; the body's @Refinement mis-attached to
            // the enclosing package).
            "dependency_usage" => {
                self.process_dependency_usage(node, parent_id, parent_key, result)
            }
            "requirement_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::RequirementUsage,
            ),
            "constraint_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConstraintUsage,
            ),
            "flow_connection_usage" => {
                let kind = self.flow_connection_usage_kind(node);
                self.process_usage(node, parent_id, parent_key, result, kind)
            }
            // Message (SysML.xtext Message:1240 → FlowUsage with isMessage).
            // Peeled from the shared kerml_usage bundle so its `of` payload and
            // from/to ends parse; process_usage's FlowUsage endpoint extractor
            // captures the `message_ends` source/target — the SAME extractor and
            // source/target props a `flow`'s own ends use (one home).
            //
            // Residual (successor lane, NOT message-specific): the spec's fuller
            // MessageDeclaration shape — from/to ends as ParameterMemberships
            // (MessageEventMember) each owning an EventOccurrenceUsage that
            // reference-subsets the endpoint, and the `of` payload as a
            // FeatureMembership (PayloadFeatureMember) owning the payload feature
            // (ParameterMembership and EventOccurrenceUsage DO exist as kinds;
            // MessageEventMember/PayloadFeatureMember are xtext rule names, not
            // kinds). This applies to FLOW `of`/ends AND message `of`/ends alike
            // — a flow's own `of <type>` is likewise not lowered today — so it is
            // ONE successor item covering both, not a message-only gap. The
            // from/to ends land now as resolvable source/target; the `of`
            // payload parses but is not yet lowered (same as a flow's `of`).
            "message_usage" => {
                let (id, ck) =
                    self.process_usage(node, parent_id, parent_key, result, ElementKind::FlowUsage)?;
                if let Some(elem) = result.graph.get_element_mut(&id) {
                    elem.set_prop("isMessage", true);
                }
                Some((id, ck))
            }
            "kerml_usage" => {
                let kind = self.kerml_usage_kind(node);
                let is_message = kind == ElementKind::FlowUsage;
                let (id, _ck) = self.process_usage(node, parent_id, parent_key, result, kind)?;
                if is_message {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("isMessage", true);
                    }
                }
                Some((id, _ck))
            }
            // TS-1.3 gap #7 / TS-1.7 gap #13: KerML keyword-driven definitions
            // (`metaclass`, `class`, `struct`, `datatype`, `function`, `behavior`,
            // `interaction`, `assoc`, `type`, `predicate`, `classifier`) emit
            // `kerml_definition` CST nodes. Per KerML.xtext lines 784-1060 each
            // keyword maps to a specific element kind; `assoc struct` maps to
            // `AssociationStructure`. Previously these all collapsed to the
            // generic `Definition` fallback (~508 instances corpus-wide).
            "kerml_definition" => {
                let kind = self.kerml_definition_kind(node);
                self.process_definition(node, parent_id, parent_key, result, kind)
            }
            "binding_usage" | "bind_as_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::BindingConnectorAsUsage,
            ),
            "succession_usage" => {
                // The tree-sitter grammar splits `first X then Y;` into two
                // succession_usage nodes: one with `first` keyword and one with
                // `then` keyword. Detect the `first X` half and merge it with
                // the following `then Y` sibling into a single SuccessionAsUsage.
                // `first` and `then` are anonymous string-literal nodes in
                // the grammar, so check the node text instead of named_children.
                let node_text = self.node_text(node);
                let has_first_keyword = node_text.trim_start().starts_with("first");

                if has_first_keyword {
                    // This is the `first X` half of `first X then Y;`.
                    // Extract the source name and check if the next sibling is
                    // a `then Y` succession_usage.
                    let source_name = self
                        .find_child_text(node, "feature_chain")
                        .or_else(|| self.find_child_text(node, "identifier"))
                        .map(|s| s.trim().to_owned());

                    let next = node.next_named_sibling();
                    let next_is_then = next
                        .as_ref()
                        .map(|n| {
                            n.kind() == "succession_usage"
                                && self.node_text(n).trim_start().starts_with("then")
                        })
                        .unwrap_or(false);

                    if next_is_then && source_name.is_some() {
                        // Stash the source name so the `then Y` handler can pick
                        // it up. We use a field on self rather than modifying the
                        // graph, since the `then Y` node hasn't been processed yet.
                        self.pending_first_source = source_name;
                        // Don't create an element for the `first X` half.
                        None
                    } else {
                        // Standalone `first X;` without a following `then` — create
                        // element normally (uncommon but possible).
                        let (id, _ck) = self.process_usage(
                            node,
                            parent_id,
                            parent_key,
                            result,
                            ElementKind::SuccessionAsUsage,
                        )?;
                        Some((id, _ck))
                    }
                } else {
                    // This is a `then Y` node. Check if preceded by a `first X`
                    // half (stashed source) or by a transition_usage to merge into.

                    // Check for pending `first X` source from the preceding half.
                    let first_source = self.pending_first_source.take();

                    // When `then <target>;` follows a split transition pattern
                    // (transition_usage + feature_declaration + succession_usage),
                    // merge the target into the preceding TransitionUsage instead of
                    // creating a standalone SuccessionAsUsage.
                    let merge_target = {
                        let mut sibling = node.prev_named_sibling();
                        let mut found: Option<tree_sitter::Node> = None;
                        for _ in 0..3 {
                            match sibling {
                                Some(s) if s.kind() == "transition_usage" => {
                                    found = Some(s);
                                    break;
                                }
                                // Only skip feature_declaration (trigger spillover)
                                Some(s) if s.kind() == "feature_declaration" => {
                                    sibling = s.prev_named_sibling();
                                }
                                _ => break,
                            }
                        }
                        found.and_then(|prev| {
                            let prev_start = prev.start_byte();
                            result
                                .graph
                                .elements
                                .values()
                                .find(|e| {
                                    e.kind == ElementKind::TransitionUsage
                                        && e.owner.as_ref() == parent_id.as_ref()
                                        && e.spans.iter().any(|s| s.start == prev_start)
                                })
                                .map(|e| e.id.clone())
                        })
                    };
                    if let Some(prev_id) = merge_target {
                        // Extract target name from succession_usage's feature_chain/identifier
                        if let Some(target_name) = self
                            .find_child_text(node, "feature_chain")
                            .or_else(|| self.find_child_text(node, "identifier"))
                        {
                            let target_name = target_name.trim();
                            if !target_name.is_empty() {
                                if let Some(elem) = result.graph.get_element_mut(&prev_id) {
                                    elem.set_prop("target", target_name.to_owned());
                                }
                            }
                        }
                        None
                    } else {
                        let (id, _ck) = self.process_usage(
                            node,
                            parent_id,
                            parent_key,
                            result,
                            ElementKind::SuccessionAsUsage,
                        )?;

                        // If we have a stashed source from a preceding `first X`,
                        // use it directly instead of inferring.
                        if let Some(source_name) = first_source {
                            if let Some(elem) = result.graph.get_element_mut(&id) {
                                elem.set_prop("source", source_name);
                            }
                        } else {
                            // Infer source from preceding CST sibling if not already set.
                            // `then X;` (succession_usage) only captures the target; the source
                            // is the element declared before the `then`.
                            let needs_source = result
                                .graph
                                .get_element(&id)
                                .map(|e| e.get_prop("source").is_none())
                                .unwrap_or(false);
                            if needs_source {
                                if let Some(source_name) =
                                    self.infer_succession_source(node, parent_id, result)
                                {
                                    if let Some(elem) = result.graph.get_element_mut(&id) {
                                        elem.set_prop("source", source_name);
                                    }
                                }
                            }
                        }
                        Some((id, _ck))
                    }
                }
            }
            "connector_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConnectorAsUsage,
            ),
            "succession_decl" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::SuccessionAsUsage,
            ),
            "transition_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::TransitionUsage,
            ),

            // Use case definitions and usages
            // case_def now also covers use_case_def (merged in Round 4 B4)
            "case_def" => {
                let kind = if self.has_anonymous_child(node, "use") {
                    ElementKind::UseCaseDefinition
                } else {
                    ElementKind::CaseDefinition
                };
                self.process_definition(node, parent_id, parent_key, result, kind)
            }
            // case_usage now also covers use_case_usage (merged in Round 4 B1+B2)
            "case_usage" => {
                let kind = if self.has_anonymous_child(node, "use") {
                    ElementKind::UseCaseUsage
                } else {
                    ElementKind::CaseUsage
                };
                self.process_usage(node, parent_id, parent_key, result, kind)
            }
            "include_use_case_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::IncludeUseCaseUsage,
            ),
            // Actor usage → ActorMembership wrapping a PartUsage
            "actor_usage" => self.process_actor_usage(node, parent_id, parent_key, result),

            // Stakeholder usage → StakeholderMembership wrapping a PartUsage (G08f).
            // SysML.xtext:2093-2099; mirrors actor_usage exactly.
            "stakeholder_usage" => {
                self.process_stakeholder_usage(node, parent_id, parent_key, result)
            }

            // Concern/Viewpoint defs+usages peeled out of the generic fallback (G08f):
            // both use RequirementBody (SysML.xtext:2151/2159/2399/2403).
            "concern_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConcernDefinition,
            ),
            "concern_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ConcernUsage,
            ),
            "viewpoint_def" => self.process_definition(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ViewpointDefinition,
            ),
            "viewpoint_usage" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ViewpointUsage,
            ),

            // Standalone Subsetting relationship: `subset X subsets Y;` (G08e).
            // KerML.xtext:679 — a NonFeatureElement, minted as a Subsetting relationship
            // owned by the enclosing namespace, with both endpoints resolved by name.
            "subsetting_decl" => self.process_subsetting_decl(node, parent_id, parent_key, result),

            // Satisfy vs verify requirement — the two share one CST rule
            // (`satisfy_requirement`, tree-sitter/rules/usages.js) but NOT one
            // element kind:
            //  * `satisfy …` → SatisfyRequirementUsage (SysML.xtext:2112).
            //  * `verify requirement …` → RequirementVerificationMembership
            //    owning a plain SysML::RequirementUsage — per SysML.xtext's
            //    RequirementVerificationMember / RequirementVerificationUsage
            //    (SysML.xtext:2257-2270). No isVerify marker prop; the
            //    membership kind IS the classification.
            "satisfy_requirement" => {
                if self.has_anonymous_child(node, "verify") {
                    let (mem_key, mut membership) = self.mint_direct_element(
                        parent_key,
                        parent_id,
                        ElementKind::RequirementVerificationMembership,
                        None,
                    );
                    membership.spans.push(self.node_span(node));
                    let mem_id = self.add_with_ownership_keyed(
                        membership,
                        parent_id,
                        parent_key,
                        &mem_key,
                        &mut result.graph,
                    );
                    // The verified-requirement check-usage: name + `: Req`
                    // FeatureTyping children come from the shared usage path.
                    // Return the USAGE's (id, key) — the walker parents this
                    // node's CST children (requirement_body members such as
                    // `attribute :>> x = v` redefinition bindings) under the
                    // returned element, and those belong to the check-usage,
                    // not the membership.
                    return self.process_usage(
                        node,
                        &Some(mem_id),
                        Some(&mem_key),
                        result,
                        ElementKind::RequirementUsage,
                    );
                }
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::SatisfyRequirementUsage,
                )?;
                // Extract direct reference (e.g., `satisfy MaxSpeed;` → ref: identifier)
                // This is the requirement being satisfied, stored as unresolved_type.
                if let Some(ref_text) = self.find_field_text(node, "ref") {
                    let ref_text = ref_text.trim();
                    if !ref_text.is_empty() {
                        if let Some(elem) = result.graph.get_element_mut(&id) {
                            elem.set_prop("unresolved_type", ref_text.to_owned());
                        }
                    }
                }
                // Extract "by <subject>" clause
                if let Some(subject) = self.find_field_text(node, "subject") {
                    let subject = subject.trim();
                    if !subject.is_empty() {
                        if let Some(elem) = result.graph.get_element_mut(&id) {
                            elem.set_prop("satisfiedBy", subject.to_owned());
                        }
                    }
                }
                Some((id, _ck))
            }

            // Subject declaration inside requirement body → SubjectMembership
            "subject_requirement" => {
                self.process_subject_requirement(node, parent_id, parent_key, result)
            }

            // Objective requirement (inside case bodies) → ObjectiveMembership
            "objective_requirement" => {
                self.process_objective_requirement(node, parent_id, parent_key, result)
            }

            // Verify constraint (inside objective bodies) → RequirementVerificationMembership
            "verify_constraint" => {
                self.process_verify_constraint(node, parent_id, parent_key, result)
            }

            // Assume/require/frame constraint inside requirement body
            "assume_constraint" => {
                self.process_requirement_constraint(node, parent_id, parent_key, result, "assume")
            }
            "require_constraint" => {
                self.process_requirement_constraint(node, parent_id, parent_key, result, "require")
            }
            // Reference form: `require existingUsage;` / `assume …;` —
            // membership pointing at an existing requirement, no new content.
            "require_referenced" => self.process_referenced_requirement_constraint(
                node, parent_id, parent_key, result, "require",
            ),
            "assume_referenced" => self.process_referenced_requirement_constraint(
                node, parent_id, parent_key, result, "assume",
            ),
            "frame_constraint" => {
                self.process_requirement_constraint(node, parent_id, parent_key, result, "frame")
            }

            // Special action usages (tree-sitter node names differ from ElementKind)
            "accept_action" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::AcceptActionUsage,
                )?;
                // TS-2.14 accept payloadParameter: mint a ParameterMembership
                // wrapping a ReferenceUsage that mirrors the accept's name +
                // typing. We keep the name on the AcceptActionUsage itself
                // (per `test_accept_action_dispatch` invariant) and also
                // mint the child wrapper so the spec `payloadParameter`
                // slot is reachable.
                // Grammar: `seq("accept", optional(field("name", _name)),
                //               optional($.typing), ";")`.
                self.emit_accept_payload_parameter(node, &id, &ck, &mut result.graph);
                // RSC port-flow Wave B-inc-2: capture the `via <port>` receiving
                // port (grammar `via_port` field) so the runtime action compiler
                // lowers it to an Accept node `port_source` (mirror of the SM/L38
                // trigger `via_port` capture).
                if let Some(port) = self.find_field_text(node, "via_port") {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("via_port", port);
                    }
                }
                Some((id, ck))
            }
            "send_action" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::SendActionUsage,
                )?;
                // TS-2.14 send payloadArgument: emit the first _expression
                // after `send` as a structured expression subtree wrapped in
                // a ParameterMembership so `SendActionUsage.payloadArgument`
                // is a reachable child slot.
                // Grammar: `seq("send", $._expression,
                //   optional(seq(choice("to","via"), $._expression)), ";")`.
                self.emit_send_payload_argument(node, &id, &ck, &mut result.graph);
                // L26 SM-send: capture the `via <port>` target so the runtime SM
                // compiler can lower it to an addressed MessageTransfer. The
                // grammar isolates the target as the 2nd named child but with no
                // field, so it would otherwise be dropped (mirror of the L38
                // trigger `via_port` drop).
                if let Some(port) = self.send_via_port_text(node) {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("via_port", port.to_owned());
                    }
                }
                Some((id, ck))
            }
            "assignment_action" => {
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::AssignmentActionUsage,
                )?;
                self.extract_assignment_props(node, &id, &mut result.graph);
                Some((id, _ck))
            }
            // TerminateNode (SysML.xtext:1636) → TerminateActionUsage.
            // The direct forms `terminate;` / `terminate <ref>;` materialize the
            // distinct kind that the runtime already lowers to
            // `ActionNodeIR::Terminate` (actions/mod.rs). A terminate node has no
            // element-level name in these forms; the optional argument names the
            // terminated occurrence and is captured BOTH as the flat
            // `unresolved_target` string (resolved additively to
            // `resolvedTerminatedOccurrence` by elaborate::actions) AND in the
            // spec `NodeParameterMember` shape — a ReferenceUsage slot child
            // carrying the FeatureBinding expression subtree (SysML.xtext
            // `NodeParameterMember`/`NodeParameter`/`FeatureBinding`; vocab
            // `terminatedOccurrenceArgument`). The runtime terminate node
            // ignores the target today (it ends the containing performance), so
            // this is faithful capture, not invented semantics.
            "terminate_action" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::TerminateActionUsage,
                )?;
                if let Some(target) = self.find_field_text(node, "target") {
                    let target = target.trim();
                    if !target.is_empty() {
                        if let Some(elem) = result.graph.get_element_mut(&id) {
                            elem.set_prop("unresolved_target", target.to_owned());
                        }
                    }
                }
                self.emit_terminate_node_parameter(node, &id, &ck, &mut result.graph);
                // A terminate node carries no declared name; the default usage
                // name extraction can pick up the target reference identifier, so
                // clear it (mirrors the for_action name-clearing rationale).
                if let Some(elem) = result.graph.get_element_mut(&id) {
                    elem.name = None;
                    elem.name_span = None;
                }
                Some((id, ck))
            }
            "if_action" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::IfActionUsage,
                )?;
                // TS-2.14 if-branch payload: mint the condition argument as
                // a structured expression subtree under the IfActionUsage so
                // `IfActionUsage.ifArgument` becomes a reachable child slot.
                // Grammar: `seq("if", $._expression, $.action_body, ...)`.
                self.emit_if_argument(node, &id, &ck, &mut result.graph);
                Some((id, ck))
            }
            "while_action" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::WhileLoopActionUsage,
                )?;
                // TS-2.14 while-loop payload: emit the loop condition as a
                // structured expression subtree so `WhileLoopActionUsage.
                // whileArgument` is reachable as a child slot.
                // Grammar: `seq(choice("while","until"), $._expression, $.action_body)`.
                self.emit_while_argument(node, &id, &ck, &mut result.graph);
                Some((id, ck))
            }
            "for_action" => {
                // G14: the for_action grammar has `field("var", _name)` and no
                // `field("name", ...)`. The default name extraction falls back
                // to `find_child_text(node, "identifier")` which can pick up
                // the var (or the var's tail when error-recovery promotes a
                // type name into the var slot — e.g. `for n : ScalarValues::
                // Integer in (..)` lands `Integer` as the var identifier).
                // ForLoopActionUsage has no element-level name in the spec,
                // so clear whatever the default extraction picked up.
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::ForLoopActionUsage,
                )?;
                if let Some(elem) = result.graph.get_element_mut(&id) {
                    elem.name = None;
                    elem.name_span = None;
                }
                // TS-2.14 for-loop loopVariable + seqArgument:
                // mint a ParameterMembership wrapping a ReferenceUsage for
                // the var, and a structured expression subtree for the
                // sequence after `in`. Grammar:
                // `seq("for", field("var", _name), "in", _expression, action_body)`.
                self.emit_for_loop_variable(node, &id, &ck, &mut result.graph);
                self.emit_for_seq_argument(node, &id, &ck, &mut result.graph);
                Some((id, ck))
            }

            // Control-flow nodes
            "control_flow_node" => {
                let keyword = node
                    .child_by_field_name("keyword")
                    .map(|k| k.kind().to_owned());
                let kind = match keyword.as_deref() {
                    Some("merge") => ElementKind::MergeNode,
                    Some("decide") => ElementKind::DecisionNode,
                    Some("fork") => ElementKind::ForkNode,
                    Some("join") => ElementKind::JoinNode,
                    Some("perform") => ElementKind::PerformActionUsage,
                    _ => ElementKind::MergeNode,
                };
                let (id, _ck) =
                    self.process_usage(node, parent_id, parent_key, result, kind.clone())?;
                // Generate synthetic names for anonymous control-flow nodes so that
                // succession_usage (`then X;`) can reference them by name.
                if let Some(elem) = result.graph.get_element(&id) {
                    if elem.name.is_none() {
                        let (counter_idx, prefix) = match kind {
                            ElementKind::ForkNode => (0, "$fork"),
                            ElementKind::JoinNode => (1, "$join"),
                            ElementKind::MergeNode => (2, "$merge"),
                            ElementKind::DecisionNode => (3, "$decide"),
                            _ => (0, "$ctrl"),
                        };
                        let n = self.anon_control_counters[counter_idx];
                        self.anon_control_counters[counter_idx] += 1;
                        let synthetic_name = format!("{}_{}", prefix, n);
                        if let Some(elem) = result.graph.get_element_mut(&id) {
                            elem.name = Some(synthetic_name);
                        }
                    }
                }
                Some((id, _ck))
            }

            // Generic definition fallback (analysis, verification, view, etc.) with
            // keyword field. G08f (closed): `concern`/`viewpoint` were peeled into
            // dedicated requirement_body rules (concern_def/viewpoint_def) and
            // `stakeholder def` was removed (no StakeholderDefinition in the spec),
            // so the keyword set here is {analysis, verification, view, rendering,
            // metadata} — all mapped; the `unwrap_or` is a defensive default.
            "definition" => {
                let kind = node
                    .child_by_field_name("keyword")
                    .and_then(|k| match self.node_text(&k) {
                        "analysis" => Some(ElementKind::AnalysisCaseDefinition),
                        "verification" => Some(ElementKind::VerificationCaseDefinition),
                        "view" => Some(ElementKind::ViewDefinition),
                        "rendering" => Some(ElementKind::RenderingDefinition),
                        "metadata" => Some(ElementKind::MetadataDefinition),
                        _ => None,
                    })
                    .unwrap_or(ElementKind::Definition);
                self.process_definition(node, parent_id, parent_key, result, kind)
            }

            // Generic usage fallback (calc, case, analysis, etc.) with keyword field.
            // G08 audit (2026-05-26): grammar (rules/usages.js:434) constrains the
            // keyword field to a closed, fully-mapped set so the `unwrap_or(Usage)`
            // branch is structurally unreachable. Left as a defensive default.
            "usage" => {
                let kind = node
                    .child_by_field_name("keyword")
                    .and_then(|k| match self.node_text(&k) {
                        "calc" => Some(ElementKind::CalculationUsage),
                        "analysis" => Some(ElementKind::AnalysisCaseUsage),
                        "verification" => Some(ElementKind::VerificationCaseUsage),
                        "view" => Some(ElementKind::ViewUsage),
                        "rendering" => Some(ElementKind::RenderingUsage),
                        "metadata" => Some(ElementKind::MetadataUsage),
                        // concern/viewpoint peeled to dedicated rules (G08f);
                        // exhibit peeled to exhibit_state_usage (G23).
                        _ => None,
                    })
                    .unwrap_or(ElementKind::Usage);
                self.process_usage(node, parent_id, parent_key, result, kind)
            }

            // Succession flow usage
            "succession_flow_usage" => {
                self.process_usage(node, parent_id, parent_key, result, ElementKind::FlowUsage)
            }

            // Assert constraint usage (standalone rule)
            "assert_constraint_usage" => {
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::AssertConstraintUsage,
                )?;
                // Extract isNegated from "not" anonymous child (e.g. "assert not constraint ...")
                if self.has_anonymous_child(node, "not") {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("isNegated", Value::Bool(true));
                    }
                }
                Some((id, _ck))
            }

            // Return feature inside calc/function bodies
            "return_feature" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ReturnParameterMembership,
            ),

            // Invariant constraint (inv [name] { expr })
            // Per KerML.xtext:976 — `Invariant returns SysML::Invariant`.
            // Pest emits `ElementKind::Invariant` for this form
            // (see sysml-parser-batch/src/ast/mod.rs:455). Mirror that here
            // to close G12 parser-equivalence gap. We retain the `role` prop
            // for any consumers still keyed on the previous shape.
            "inv_constraint" => {
                let (id, _ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::Invariant,
                )?;
                if let Some(elem) = result.graph.get_element_mut(&id) {
                    elem.set_prop("role", Value::String("invariant".to_owned()));
                }
                Some((id, _ck))
            }

            // Target transition usage (guard/trigger then <target> inside state bodies)
            //
            // When this node follows a transition_usage sibling (possibly with
            // intervening feature_declaration or succession_usage siblings from
            // grammar ambiguity), merge the target/guard/trigger props into that
            // existing TransitionUsage instead of creating a duplicate element.
            "target_transition_usage" => {
                // Walk backwards through siblings to find nearest transition_usage.
                // The grammar sometimes splits a transition into:
                //   transition_usage + feature_declaration + target_transition_usage
                // when trigger_action fails to capture the trigger name.
                let prev_transition_id = {
                    let mut sibling = node.prev_named_sibling();
                    let mut found_transition: Option<tree_sitter::Node> = None;
                    // Walk back at most 3 siblings (transition, feature_decl, succession)
                    for _ in 0..3 {
                        match sibling {
                            Some(s) if s.kind() == "transition_usage" => {
                                found_transition = Some(s);
                                break;
                            }
                            Some(s) => sibling = s.prev_named_sibling(),
                            None => break,
                        }
                    }
                    found_transition.and_then(|prev| {
                        let prev_start = prev.start_byte();
                        result
                            .graph
                            .elements
                            .values()
                            .find(|e| {
                                e.kind == ElementKind::TransitionUsage
                                    && e.owner.as_ref() == parent_id.as_ref()
                                    && e.spans.iter().any(|s| s.start == prev_start)
                            })
                            .map(|e| e.id.clone())
                    })
                };

                if let Some(prev_id) = prev_transition_id {
                    // Merge into the previous TransitionUsage. The previous
                    // transition's CanonicalKey is no longer in hand, so
                    // emit_transition_features derives child keys from the
                    // (canonical) transition id.
                    self.extract_target_transition_props(node, &prev_id, &mut result.graph);
                    self.emit_transition_features(node, &prev_id, None, &mut result.graph);
                    None
                } else {
                    // Standalone target_transition_usage (e.g. `if guard then target;`)
                    let (id, _ck) = self.process_usage(
                        node,
                        parent_id,
                        parent_key,
                        result,
                        ElementKind::TransitionUsage,
                    )?;
                    self.extract_target_transition_props(node, &id, &mut result.graph);
                    Some((id, _ck))
                }
            }

            // Import/alias/expose/filter
            "import_decl" => self.process_import(node, parent_id, parent_key, result),
            "expose_decl" => self.process_expose(node, parent_id, parent_key, result),
            "alias_decl" => self.process_alias(node, parent_id, parent_key, result),
            // G09 — ElementFilterMembership (SysML.xtext:229-232,
            // routed via PackageBody:203 / PackageBodyElement:213).
            "filter_decl" => self.process_filter(node, parent_id, parent_key, result),

            // Render usage in view bodies → ViewRenderingMembership
            "render_usage" => self.process_render_usage(node, parent_id, parent_key, result),

            // Prefix metadata annotation: `#keyword` immediately preceding
            // its annotated declaration (G24 / SysML §7.27.4).
            "prefix_metadata_annotation" => {
                self.process_prefix_metadata_annotation(node, parent_id, parent_key, result)
            }

            // Metadata annotation: @TypeRef { ... } or @TypeRef ;
            //
            // G16: metadata-usage-property-lowering. The type reference is
            // stored under `unresolvedTypeName` / `annotationType`. Runtime
            // consumers (`metadata.rs::is_metadata_typed_as`, `compiler.rs`'s
            // @DataSource / @ToolVariable lookups) all match on
            // `unresolvedTypeName`, so the element stays ANONYMOUS — the
            // `@Type` annotation form declares no member name per spec, and
            // the old bare-type-suffix name fallback collided with the
            // metadata def itself in the same scope (S001 false positive,
            // coffee-machine fixture triage).
            "metadata_usage" => {
                let (id, ck) = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::MetadataUsage,
                )?;
                // Extract the type reference from the "type" field
                if let Some(type_text) = self.find_field_text(node, "type") {
                    let type_text = type_text.trim().to_owned();
                    if !type_text.is_empty() {
                        if let Some(elem) = result.graph.get_element_mut(&id) {
                            elem.set_prop("annotationType", type_text.clone());
                            elem.set_prop("unresolvedTypeName", type_text.clone());
                        }
                        // Mint a real FeatureTyping edge to the metadata def —
                        // the same structure `part x : Foo` gets from its
                        // `typing` CST node. Routes the reference through
                        // pass-1 resolution + the InheritanceIndex so
                        // `:>> attr` redefinitions inside the usage body
                        // resolve against the metadata def's attributes
                        // (coffee-machine triage: E200 false positives).
                        let span = node.child_by_field_name("type").map(|n| self.node_span(&n));
                        create_feature_typing_with_key(
                            &mut result.graph,
                            id.clone(),
                            type_text,
                            span,
                            &ck,
                            "typing",
                            0,
                        );
                    }
                }
                Some((id, ck))
            }

            // State body subactions: entry/do/exit → ActionUsage with stateSubactionKind
            "entry_action" => {
                self.process_state_subaction(node, parent_id, parent_key, result, "entry")
            }
            "do_action" => self.process_state_subaction(node, parent_id, parent_key, result, "do"),
            "exit_action" => {
                self.process_state_subaction(node, parent_id, parent_key, result, "exit")
            }

            // Inline transition chain inside state bodies
            "state_transition_chain" => {
                self.process_state_transition_chain(node, parent_id, parent_key, result)
            }

            // Send inside state body (entry/do/exit send_inline handled as children)
            "send_inline" => {
                let res = self.process_usage(
                    node,
                    parent_id,
                    parent_key,
                    result,
                    ElementKind::SendActionUsage,
                );
                if let Some((id, ck)) = &res {
                    // A-scalar: project the payload expression subtree (the first
                    // named child) as the send's payloadArgument, mirroring the
                    // statement-form `send_action` arm. Without this the inline
                    // `entry send <payload> via <port>` dropped its payload
                    // entirely — only the statement form projected it.
                    self.emit_send_payload_argument(node, id, ck, &mut result.graph);
                    // L26 SM-send: capture `via <port>` (2nd named child, no field)
                    // so the SM compiler lowers it to an addressed MessageTransfer.
                    if let Some(port) = self.send_via_port_text(node) {
                        if let Some(elem) = result.graph.get_element_mut(id) {
                            elem.set_prop("via_port", port.to_owned());
                        }
                    }
                }
                res
            }

            // Comment and documentation annotations
            "comment_element" => {
                self.process_comment(node, parent_id, parent_key, result, ElementKind::Comment)
            }
            "doc_comment" => self.process_comment(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::Documentation,
            ),

            // KerML TextualRepresentation (`rep? language "L" /* body */`): a
            // distinct AnnotatingElement carrying a `language` + `body`
            // (KerML.xtext `TextualRepresentation returns SysML::TextualRepresentation`).
            // Without this arm it fell through to `_ => None`, dropping the
            // language/body (registry: tree-sitter.textual-representation-generic-lowering).
            "textual_representation" => {
                self.process_textual_representation(node, parent_id, parent_key, result)
            }

            // TS-1.3 gap #3: orphan `feature_declaration` (no preceding split-able
            // sibling). Per SysML spec (SysML.xtext line 627:
            // `DefaultReferenceUsage returns SysML::ReferenceUsage`), keyword-
            // less feature usages such as `in x : Type;` / `out y : Type;`
            // and metadata-body members (`@DataSource { path = "a.csv"; }`,
            // the `MetadataBodyUsage returns SysML::ReferenceUsage` production
            // at SysML.xtext line 173) map to ReferenceUsage. The split-decl
            // guard above (with the TS-1.4 metadata-body augment exception)
            // catches the augment cases; this arm is the standalone fallback
            // that used to fall through to `_ => None`.
            "feature_declaration" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ReferenceUsage,
            ),

            // G04b: anonymous typed parameter (`in : Real[1];`, no name) inside a
            // calc_body. Same lowering as a nameless feature_declaration →
            // ReferenceUsage; the `: Type` lands as a FeatureTyping via
            // create_usage_rels, closing the KerML Vector/Tensor FeatureTyping floor.
            "anonymous_typed_param" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ReferenceUsage,
            ),

            // TS-1.4 gap #4: orphan `feature_redefinition` inside usage bodies
            // (`attribute durationPF { :>> quantity = isq.T; :>> exponent = 1; }`).
            // Each `:>> name = expr` line is a standalone redefining usage
            // whose default_value carries the bulk of FeatureReferenceExpression,
            // LiteralInteger and OperatorExpression elements Pest emits in
            // ISQ/USCustomary unit libraries. Process via process_usage so the
            // default_value flows through the expression-subtree pipeline; the
            // `:>` redefinition target lands as a Redefinition relationship via
            // `create_usage_rels`.
            "feature_redefinition" => self.process_usage(
                node,
                parent_id,
                parent_key,
                result,
                ElementKind::ReferenceUsage,
            ),

            // Skip structural nodes (they're just containers)
            "source_file" | "package_body" | "definition_body" | "usage_body" | "action_body"
            | "state_body" | "relationship_body" | "requirement_body" | "constraint_body"
            | "function_body" | "calc_body" | "case_body" | "enum_body" => None,

            // Skip tokens and other non-semantic nodes
            _ => None,
        }
    }
}
