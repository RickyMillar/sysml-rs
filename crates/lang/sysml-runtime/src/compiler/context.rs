//! `ModelCompiler` execution-context construction: seed extraction,
//! ISQ tagging, overrides, and value parsing.

use std::sync::Arc;

use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};

use crate::expressions::EvalContext;

use super::*;

/// Build an [`EvalContext`] from a [`ModelGraph`].
///
/// Extracts attribute values from elements so that constraint and verification
/// evaluation can reference them by name. Attaches the graph for lazy
/// feature-chain resolution of `Value::Ref` entries.
///
/// Also performs ISQ auto-tagging: if an attribute's type annotation matches
/// an ISQ quantity type (e.g., `LengthValue`, `MassValue`), the numeric default
/// is automatically wrapped as `Value::Quantity` with the corresponding
/// dimension vector from `sysml_core::physics::ISQ_TYPES`.
pub fn context_from_graph(graph: &Arc<ModelGraph>) -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.graph = Some(Arc::clone(graph));

    for element in graph.elements.values() {
        // Skip expression-AST element kinds — these carry `name` copied from
        // the source identifier (e.g. a `FeatureReferenceExpression` named
        // "bimetalTemp") but are not value-carrying model features. Binding
        // them into the context shadows the real AttributeUsage with the
        // same name and causes infinite recursion in `try_eval_unresolved`.
        if is_expression_ast_kind(&element.kind) {
            continue;
        }
        // Skip calc-def-internal (invocation-scoped) features — see
        // `is_calc_scoped_seed_feature`. Keeps a calc `return B`/`in Bs` from
        // shadowing a model attribute under its bare name.
        if is_calc_scoped_seed_feature(graph, element) {
            continue;
        }
        if let Some(name) = &element.name {
            // Bind "value" property.
            if let Some(val) = element.get_prop("value") {
                let val = maybe_tag_isq(graph, element, val.clone());
                ctx.set(name.clone(), val);
                continue;
            }
            // Bind "default" values (only if "value" not already set).
            if let Some(val) = element.get_prop("default") {
                let val = maybe_tag_isq(graph, element, val.clone());
                ctx.set(name.clone(), val);
                continue;
            }
            // Check literal children: the parser creates child LiteralInteger/
            // LiteralRational elements for `attribute x = 85` rather than setting
            // a "value" property on the attribute itself.
            let mut found_literal = false;
            for child in graph.children_of(&element.id) {
                if matches!(
                    child.kind,
                    ElementKind::LiteralInteger
                        | ElementKind::LiteralRational
                        | ElementKind::LiteralBoolean
                        | ElementKind::LiteralString
                ) {
                    if let Some(val) = child.get_prop("value") {
                        let val = maybe_tag_isq(graph, element, val.clone());
                        ctx.set(name.clone(), val);
                        found_literal = true;
                        break;
                    }
                }
            }
            // Store as Ref for lazy feature chain resolution, but only if no
            // concrete value has already been bound under this name by an
            // earlier iteration. SysML allows the same feature name to appear
            // on a definition (no value) and a usage (with a value, e.g.
            // `attribute temperature = 180` redefining `attribute temperature :
            // Real`). BTreeMap iteration is keyed by ElementId UUID, so the
            // value-bearing usage may be visited before the value-less
            // definition; without this guard the definition's Ref overwrites
            // the usage's concrete value and constraint evaluation degrades
            // to UndefinedVariable for an attribute the model actually does
            // bind. Concrete values are sticky; only Ref-with-Ref is
            // overwritten so the latest-seen element id wins for chaining.
            if !found_literal {
                let new_val = Value::Ref(element.id.clone());
                match ctx.get(name) {
                    Some(existing) if !matches!(existing, Value::Ref(_)) => {
                        // Keep the concrete value already bound; don't shadow with a Ref.
                    }
                    _ => {
                        ctx.set(name.clone(), new_val);
                    }
                }
            }
        }
    }
    ctx.occurrence_registry = Some(std::sync::Arc::new(std::sync::Mutex::new(
        sysml_core::occurrence::OccurrenceRegistry::new(),
    )));
    ctx
}

/// True for element kinds produced by the expression AST builder. These are
/// structural nodes inside an expression subtree (operator/literal/feature-ref
/// expression elements), not value-carrying features. They are excluded from
/// `context_from_graph` so that identifiers inside expressions don't shadow
/// the real attribute bindings they refer to.
pub(crate) fn is_expression_ast_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::LiteralBoolean
            | ElementKind::LiteralInteger
            | ElementKind::LiteralRational
            | ElementKind::LiteralString
            | ElementKind::LiteralInfinity
            | ElementKind::LiteralExpression
            | ElementKind::NullExpression
            | ElementKind::OperatorExpression
            | ElementKind::InvocationExpression
            | ElementKind::FeatureReferenceExpression
            | ElementKind::FeatureChainExpression
            | ElementKind::SelectExpression
            | ElementKind::CollectExpression
            | ElementKind::IndexExpression
            | ElementKind::MetadataAccessExpression
            | ElementKind::ConstructorExpression
    )
}

/// True if `element` is a calc-definition-internal feature — a parameter,
/// return parameter, or local — owned by a `CalculationDefinition`.
///
/// Per KerML §8.3.4.6.4 (`ParameterMembership` is owned by a `Behavior`/`Step`
/// /constructor result) and §8.3.4.7.8 (`ReturnParameterMembership` names the
/// result parameter of a `Function`/`Expression`), these features are
/// **invocation-scoped**: they exist while a calc is evaluated, not as
/// model-level bindings in the global namespace. The calc registry evaluates
/// calcs by invocation, so nothing legitimately reads them by bare name from
/// the runtime `EvalContext`.
///
/// They must therefore be excluded from the `EvalContext` seed walks. Binding
/// them under their bare name shadows a same-named model-level attribute — the
/// `B` collision that froze the legacy oscillator fixture: the ODE state variable `out attribute B`
/// (`CoreODE.sysml`) vs. the calc-def `return B` of `BFieldAscending`
/// (`BHModel.sysml`), where the calc return won the flat-map collision as a
/// lazy `Value::Ref` and every reader of `"B"` (the oscillator SM guard) got
/// the saturation value instead of the integrated state.
///
/// RSC-3.7 amendment §A (core-steward consult 2026-06-30) — CONFORMS-REQUIRED.
pub fn is_calc_scoped_seed_feature(graph: &ModelGraph, element: &Element) -> bool {
    if matches!(element.kind, ElementKind::ReturnParameterMembership) {
        return true;
    }
    element
        .owner
        .as_ref()
        .and_then(|oid| graph.elements.get(oid))
        .is_some_and(|owner| matches!(owner.kind, ElementKind::CalculationDefinition))
}

/// If `element` is an `AttributeUsage` typed as an ISQ quantity type, wrap
/// a numeric `Value` in `Value::Quantity` with the matching dimension vector.
///
/// Non-numeric values or unrecognized type names pass through unchanged.
///
/// RSC-5.1 (M2): this is the single home for ISQ value-tagging — `sysml-ide-db`'s
/// `eval_context_seed` delegates here rather than keeping an identical copy
/// (the dual source-of-truth the design doc flagged). Mirrors
/// [`infer_m_ref`], which shares `resolve_attribute_type_name`.
/// Both this tagging and the seed-time call will be retired once the slot mRef
/// reconstitutes `Value::Quantity` at the eval boundary (a later 5.1 step).
pub fn maybe_tag_isq(graph: &ModelGraph, element: &sysml_core::Element, val: Value) -> Value {
    // Only auto-tag AttributeUsage elements
    if element.kind != ElementKind::AttributeUsage {
        return val;
    }
    // Only wrap numeric values (Float or Int → Quantity)
    let numeric = match &val {
        Value::Float(f) => *f,
        Value::Int(n) => *n as f64,
        // Already a Quantity — don't double-wrap
        Value::Quantity { .. } => return val,
        _ => return val,
    };

    // Resolve the type name from the element
    let type_name = resolve_attribute_type_name(graph, element);
    let type_name = match type_name {
        Some(tn) => tn,
        None => return val,
    };

    // Look up in ISQ_TYPES table
    for &(isq_name, ref dim, _) in sysml_core::physics::ISQ_TYPES {
        if isq_name == type_name {
            return Value::quantity(numeric, dim.clone(), None);
        }
    }

    val
}

/// Resolve the ISQ type name from an attribute's type annotations.
///
/// Checks (in order):
/// 1. `unresolvedTypeName` property on the element
/// 2. `FeatureTyping` children with `unresolved_type` property
/// 3. Outgoing `TypeOf` relationships to named elements
///
/// `pub(crate)` so the flows layer (`extract_port_feature`) can derive a port
/// feature's payload type from the same source `infer_m_ref` reads (RSC-3.3a
/// D3 / the "value dead-end" close-out) — one home for declared-type-name
/// resolution, two callers.
pub(crate) fn resolve_attribute_type_name(
    graph: &ModelGraph,
    element: &sysml_core::Element,
) -> Option<String> {
    // Strategy 1: Direct property
    if let Some(Value::String(s)) = element.get_prop("unresolvedTypeName") {
        // Strip package prefix (e.g., "ISQ::LengthValue" → "LengthValue")
        let name = s.rsplit("::").next().unwrap_or(s);
        return Some(name.to_string());
    }

    // Strategy 2: FeatureTyping children
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping {
            if let Some(Value::String(s)) = child.get_prop("unresolved_type") {
                let name = s.rsplit("::").next().unwrap_or(&s);
                return Some(name.to_string());
            }
        }
    }

    // Strategy 3: Outgoing TypeOf relationships
    for rel in graph.outgoing(&element.id) {
        if rel.kind == RelationshipKind::TypeOf {
            if let Some(target) = graph.get_element(&rel.target) {
                if let Some(ref name) = target.name {
                    return Some(name.clone());
                }
            }
        }
    }

    None
}

/// Infer the [`MeasurementRef`](crate::slots::MeasurementRef) carried by an
/// attribute declaration, if any. **The single home** for mRef inference
/// (RSC-5.1 D-5.0.3 / RSC-5.2): slot mint (`mint_slot_store`) and the boundary
/// diagnostics (`quantity_health`) both call this — no duplicate path.
///
/// Resolution order:
///  1. **Path #1 (D-5.0.5):** an explicit `[unit]` measurement reference on the
///     declared value — recorded by the parser as the `unit` prop — wins over
///     the ISQ type. Resolved through the unit table so the mRef carries the
///     real scale/offset (the slot magnitude stays in the declared unit; the
///     scale converts it to SI). A qualified ref like `SI::kg` strips to `kg`.
///  2. **Path #2:** the attribute's ISQ type (SI-base, unit-less, scale 1).
///
/// Returns `None` for non-`AttributeUsage` elements and untyped/non-ISQ
/// attributes — the boundary diagnostics treat `None` as "no facts", never a
/// hard error (D-5.0.8 two-tier severity).
pub(crate) fn infer_m_ref(
    graph: &ModelGraph,
    decl: &ElementId,
) -> Option<crate::slots::MeasurementRef> {
    let element = graph.get_element(decl)?;
    if element.kind != ElementKind::AttributeUsage {
        return None;
    }
    if let Some(Value::String(unit_ref)) = element.get_prop("unit") {
        let bare = unit_ref.rsplit("::").next().unwrap_or(unit_ref.as_str());
        if let Some(entry) = crate::expressions::units::lookup_unit(bare) {
            return Some(crate::slots::MeasurementRef {
                dimension: entry.dimension,
                unit: Some(std::sync::Arc::from(bare)),
                scale: entry.scale,
                offset: entry.offset,
            });
        }
    }
    let type_name = resolve_attribute_type_name(graph, element)?;
    for &(isq_name, ref dim, _) in sysml_core::physics::ISQ_TYPES {
        if isq_name == type_name {
            return Some(crate::slots::MeasurementRef {
                dimension: dim.clone(),
                unit: None,
                scale: 1.0,
                offset: 0.0,
            });
        }
    }
    None
}

/// One feature reachable from a scope's owner-tree, yielded in owner-scoped
/// resolution order. See [`owner_scoped_features`].
pub(crate) struct ScopedFeature {
    /// Name of the scope (owner / ancestor) this feature is declared in, if it
    /// has one — used by the slot binder to build the instance-prefixed
    /// spelling (`{scope}.{name}`). The static quantity scan ignores it.
    pub scope_name: Option<String>,
    /// The feature's own (bare) name.
    pub name: String,
    /// The feature element's id.
    pub id: ElementId,
}

/// Walk `scope_root` and its ancestors (nearest-first), yielding every named
/// `AttributeUsage`/`ReferenceUsage` child in deterministic (name-sorted)
/// per-scope order.
///
/// This is the **graph-only structural core** of owner-scoped name resolution
/// (RSC-3.1 D-3.0.6-B): a bare feature name inside an expression resolves to
/// the nearest enclosing scope that declares it. The single home for the
/// owner-scope walk — both the constraint slot aliaser
/// ([`owner_scoped_slot_aliases`]) and the static quantity boundary diagnostics
/// ([`crate::quantity_health`]) consume it, so the walk is not re-implemented a
/// third time.
///
/// No deduplication is applied — consumers layer their own nearest-wins policy
/// on the visitation order (the slot binder dedups on the first slot hit; the
/// quantity scan dedups on first occurrence per name).
pub(crate) fn owner_scoped_features(
    graph: &ModelGraph,
    scope_root: &ElementId,
) -> Vec<ScopedFeature> {
    let mut scopes: Vec<&ElementId> = vec![scope_root];
    let ancestors = sysml_core::query::ancestors(graph, scope_root);
    scopes.extend(ancestors.iter().map(|e| &e.id));

    let mut features = Vec::new();
    for scope_id in scopes {
        let scope_name = graph.get_element(scope_id).and_then(|e| e.name.clone());
        // Sort children for deterministic ordering across builds.
        let mut children: Vec<_> = graph
            .children_of(scope_id)
            .filter(|c| {
                matches!(
                    c.kind,
                    ElementKind::AttributeUsage | ElementKind::ReferenceUsage
                )
            })
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        for child in children {
            let Some(name) = child.name.clone() else {
                continue;
            };
            features.push(ScopedFeature {
                scope_name: scope_name.clone(),
                name,
                id: child.id.clone(),
            });
        }
    }
    features
}

/// RSC-3.1 (D-3.0.6-B): collect the owner-scoped slot aliases for a
/// constraint owned by `owner_id`. Walks the owner's feature tree first
/// (its attribute children), then up its ancestor chain, mapping each
/// attribute's *bare* name to the minted slot that backs it.
///
/// An attribute named `tripped` owned by an instance `circuit5` is minted
/// as the slot `circuit5.tripped`; this resolves the bare `tripped` to
/// that slot. Both the instance-prefixed spelling (`{owner}.{attr}`) and
/// the bare spelling are tried, so top-level (unprefixed) owners and
/// multiplied-instance owners both resolve. Nearest owner wins
/// (insertion order; ancestors do not overwrite an already-mapped name).
///
/// The single home for owner-scoped constraint slot aliasing — consumed by
/// the orchestrator's compile-time slot binder and by the static
/// per-instance constraint evaluation path, so the mapping is not
/// re-implemented per consumer.
pub(crate) fn owner_scoped_slot_aliases(
    graph: &ModelGraph,
    store: &crate::slots::SlotStore,
    owner_id: &ElementId,
) -> Vec<(String, crate::slots::SlotId)> {
    let mut aliases: Vec<(String, crate::slots::SlotId)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // The owner-tree walk (owner first, then ancestors, name-sorted per
    // scope) is the shared primitive; this consumer maps each feature to a
    // minted slot, preferring the instance-prefixed spelling. A name is
    // dedup'd only once it resolves to a slot, so a slot-less nearer
    // spelling does not shadow an ancestor's minted slot.
    for f in owner_scoped_features(graph, owner_id) {
        if seen.contains(&f.name) {
            continue;
        }
        let slot = f
            .scope_name
            .as_deref()
            .and_then(|sn| store.slot_by_name(&format!("{sn}.{}", f.name)))
            .or_else(|| store.slot_by_name(&f.name));
        if let Some(slot) = slot {
            seen.insert(f.name.clone());
            aliases.push((f.name, slot));
        }
    }
    aliases
}

/// Apply key=value overrides into an [`EvalContext`].
///
/// Values are parsed as: integer, float, bool, or string (fallback).
pub fn apply_overrides(ctx: &mut EvalContext, overrides: &[(String, String)]) {
    for (key, val) in overrides {
        ctx.set(key.clone(), parse_value_string(val));
    }
}

/// Parse a single value string into a `Value`.
///
/// Heuristic: integer, float, bool (`"true"`/`"false"`), or string fallback.
pub fn parse_value_string(val: &str) -> Value {
    if let Ok(n) = val.parse::<i64>() {
        Value::Int(n)
    } else if let Ok(f) = val.parse::<f64>() {
        Value::Float(f)
    } else if val == "true" {
        Value::Bool(true)
    } else if val == "false" {
        Value::Bool(false)
    } else {
        Value::String(val.to_owned())
    }
}

// ---------------------------------------------------------------------------
// ModelCompiler
// ---------------------------------------------------------------------------

impl ModelCompiler {
    /// Detect `SampledFunction`-typed attributes in the model and return them
    /// as `(name, Value::Map)` pairs suitable for `OdeSpec::with_context_value()`.
    ///
    /// Looks for `AttributeUsage` elements whose type name (via `unresolvedTypeName`
    /// or `FeatureTyping` target name) contains "SampledFunction". Extracts
    /// ordered sample pair data from children.
    pub fn extract_sampled_functions(&self) -> Result<Vec<(String, Value)>, CompileError> {
        let mut results = Vec::new();

        for attr in self.graph.elements_by_kind(&ElementKind::AttributeUsage) {
            let attr_name = match &attr.name {
                Some(n) => n.clone(),
                None => continue,
            };

            // Check if typed as SampledFunction via multiple paths:
            // 1. unresolvedTypeName prop
            // 2. Outgoing TypeOf relationship to a SampledFunction element
            // 3. FeatureTyping child with unresolved_type containing "SampledFunction"
            // 4. Name pattern: ends with "Waveform" and has "elements" child (convention)
            let is_sf = attr
                .get_prop("unresolvedTypeName")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.contains("SampledFunction")),
                    _ => None,
                })
                .unwrap_or(false)
                || self.graph.outgoing(&attr.id).any(|rel| {
                    rel.kind == RelationshipKind::TypeOf
                        && self
                            .graph
                            .get_element(&rel.target)
                            .and_then(|e| e.name.as_deref())
                            .is_some_and(|n| n.contains("SampledFunction"))
                })
                || self.graph.children_of(&attr.id).any(|c| {
                    c.kind == ElementKind::FeatureTyping
                        && c.get_prop("unresolved_type")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s.contains("SampledFunction"))
                });

            if !is_sf {
                continue;
            }

            // Try to extract sample pairs from the "elements" property or children.
            // Format in model: elements = ((0,0), (1,16), (5,32))
            // We'll look for a property containing the pairs.
            let mut domain = Vec::new();
            let mut range = Vec::new();

            if let Some(Value::List(pairs)) = attr.get_prop("elements") {
                for pair in pairs {
                    match pair {
                        Value::List(inner) if inner.len() == 2 => {
                            if let (Some(d), Some(r)) = (
                                inner[0]
                                    .as_float()
                                    .or_else(|| inner[0].as_int().map(|i| i as f64)),
                                inner[1]
                                    .as_float()
                                    .or_else(|| inner[1].as_int().map(|i| i as f64)),
                            ) {
                                domain.push(Value::Float(d));
                                range.push(Value::Float(r));
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Also check children for SamplePair elements or "elements" attribute with tuple text
            for child in self.graph.children_of(&attr.id) {
                let dv = child
                    .get_prop("domainValue")
                    .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)));
                let rv = child
                    .get_prop("rangeValue")
                    .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)));
                if let (Some(d), Some(r)) = (dv, rv) {
                    domain.push(Value::Float(d));
                    range.push(Value::Float(r));
                }

                // AST-first: the parser emits `= ((d0,r0), (d1,r1), ...)`
                // as a nested OperatorExpression tree with operator "," —
                // outer op has one child per pair, each inner op has the
                // two literal operands. Walk that structure.
                if child.name.as_deref() == Some("elements") {
                    Self::extract_tuple_pairs_from_ast(
                        &self.graph,
                        &child.id,
                        &mut domain,
                        &mut range,
                    );
                }
            }

            // If no inline data found, check for @DataSource metadata with CSV file
            if domain.is_empty() {
                if let Some((d, r)) = self.load_data_source_csv(&attr.id)? {
                    domain = d;
                    range = r;
                }
            }

            if !domain.is_empty() {
                let mut map = std::collections::BTreeMap::new();
                map.insert(
                    "__type".to_string(),
                    Value::String("SampledFunction".to_string()),
                );
                map.insert("domain".to_string(), Value::List(domain));
                map.insert("range".to_string(), Value::List(range));
                results.push((attr_name, Value::Map(map)));
            }
        }

        Ok(results)
    }

    /// Check for `@DataSource` metadata on a SampledFunction attribute and load CSV data.
    ///
    /// Returns `Ok(None)` when the attribute carries **no** `@DataSource` child —
    /// it is simply not a CSV-backed SampledFunction. Returns `Ok(Some((domain,
    /// range)))` on success. Returns `Err` when an `@DataSource` **is** declared
    /// but cannot be honoured (no source directory to resolve against, missing
    /// `file` attribute, unreadable file, or no usable samples). Per principle #1
    /// (fail hard): a declared-but-unresolvable data source surfaces a precise
    /// error here rather than collapsing into an empty SampledFunction that
    /// detonates downstream as an opaque `RS003 unresolved runtime name __sf_*`.
    ///
    /// RSC-6.5 salsa-boundary note: this `fs::read_to_string` is **deliberately
    /// outside salsa**. `@DataSource` CSV files are not parsed SysML source, so
    /// they are not salsa `SourceFile` inputs — and their paths aren't even known
    /// until elaboration reads the `@DataSource` metadata, so they can't be
    /// declared as inputs up front. The read lives here in the runtime compiler,
    /// which is itself downstream of salsa: every `ModelCompiler`-driven build
    /// re-reads the CSV, so the data is always current for a fresh compile. The
    /// accepted tradeoff is that editing a CSV does not invalidate a salsa-cached
    /// *elaboration* (only a recompile picks it up) — acceptable because the data
    /// is consumed at orchestrator-build time, not at parse/resolve/elaborate time.
    fn load_data_source_csv(
        &self,
        attr_id: &ElementId,
    ) -> Result<Option<(Vec<Value>, Vec<Value>)>, CompileError> {
        // Find @DataSource MetadataUsage child. No child ⇒ not a CSV-backed
        // SampledFunction at all ⇒ Ok(None), no error.
        let Some(ds_meta) = self.graph.children_of(attr_id).find(|c| {
            c.kind == ElementKind::MetadataUsage
                && (c.name.as_deref() == Some("DataSource")
                    || c.get_prop("unresolvedTypeName")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == "DataSource" || s.ends_with("::DataSource")))
        }) else {
            return Ok(None);
        };

        // From here an @DataSource IS declared — any failure to honour it is a
        // hard error, not a silent skip.
        let sf_name = self
            .graph
            .get_element(attr_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| "<unnamed>".to_string());
        let Some(source_dir) = self.source_dir.as_ref() else {
            return Err(CompileError::from_message(format!(
                "@DataSource on SampledFunction '{sf_name}' cannot be resolved: no \
                 source directory configured (the project/workspace root needed to \
                 locate its CSV file)"
            )));
        };

        // Extract attributes from @DataSource children
        let mut file_path = None;
        let mut domain_col: usize = 0;
        let mut range_col: usize = 1;
        let mut has_header = true;

        for child in self.graph.children_of(&ds_meta.id) {
            let name = child.name.as_deref().unwrap_or("");
            let val = child
                .get_prop("default")
                .or_else(|| child.get_prop("value"));
            match name {
                "file" => file_path = val.and_then(|v| v.as_str().map(|s| s.to_string())),
                "domainColumn" => {
                    domain_col = val
                        .and_then(|v| {
                            v.as_int()
                                .map(|i| i as usize)
                                .or_else(|| v.as_float().map(|f| f as usize))
                        })
                        .unwrap_or(0);
                }
                "rangeColumn" => {
                    range_col = val
                        .and_then(|v| {
                            v.as_int()
                                .map(|i| i as usize)
                                .or_else(|| v.as_float().map(|f| f as usize))
                        })
                        .unwrap_or(1);
                }
                "hasHeader" => {
                    has_header = val
                        .and_then(|v| match v {
                            Value::Bool(b) => Some(*b),
                            Value::String(s) => Some(s == "true"),
                            _ => None,
                        })
                        .unwrap_or(true);
                }
                _ => {}
            }
        }

        let Some(rel_path) = file_path else {
            return Err(CompileError::from_message(format!(
                "@DataSource on SampledFunction '{sf_name}' is missing its `file` attribute"
            )));
        };
        let full_path = source_dir.join(&rel_path);

        let content = std::fs::read_to_string(&full_path).map_err(|e| {
            CompileError::from_message(format!(
                "@DataSource on SampledFunction '{sf_name}': failed to read '{}': {e}",
                full_path.display()
            ))
        })?;

        let mut domain = Vec::new();
        let mut range = Vec::new();

        for (i, line) in content.lines().enumerate() {
            if i == 0 && has_header {
                continue;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(',').collect();
            let max_col = domain_col.max(range_col);
            if fields.len() <= max_col {
                continue;
            }
            if let (Ok(d), Ok(r)) = (
                fields[domain_col].trim().parse::<f64>(),
                fields[range_col].trim().parse::<f64>(),
            ) {
                domain.push(Value::Float(d));
                range.push(Value::Float(r));
            }
        }

        if domain.is_empty() {
            return Err(CompileError::from_message(format!(
                "@DataSource on SampledFunction '{sf_name}': file '{}' yielded no \
                 usable (domain, range) samples (check delimiter, column indices, \
                 and header setting)",
                full_path.display()
            )));
        }

        Ok(Some((domain, range)))
    }

    // -- SSR auto-wiring (calc def :> GetDerivative) --------------------------

    /// Build config Maps for an ODE part definition.
    ///
    /// Returns a list of (attr_name, Map) pairs for typed config attributes.
    /// E.g., for ThermalProtectionModel with `config : ThermalProtectionConfig`, returns
    /// `[("config", Map({"bimetalResistance": 4.35, ...}))]`.
    pub(crate) fn build_config_maps(&self, ode_name: &str) -> Vec<(String, Value)> {
        let mut result = Vec::new();

        let part_def = self.graph.elements.values().find(|e| {
            e.name.as_deref() == Some(ode_name)
                && matches!(
                    e.kind,
                    ElementKind::PartDefinition | ElementKind::ActionDefinition
                )
        });
        let Some(part) = part_def else { return result };

        for child in self.graph.children_of(&part.id) {
            if child.kind != ElementKind::AttributeUsage {
                continue;
            }
            let child_name = match &child.name {
                Some(n) => n.clone(),
                None => continue,
            };
            if child
                .get_prop("default")
                .and_then(|v| v.as_float())
                .is_some()
            {
                continue;
            }

            // Find the type name from property or FeatureTyping child
            let type_name = child
                .get_prop("unresolved_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
                .or_else(|| {
                    self.graph
                        .children_of(&child.id)
                        .find(|c| c.kind == ElementKind::FeatureTyping)
                        .and_then(|c| {
                            c.get_prop("unresolved_type")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_owned())
                        })
                });
            let Some(tn) = type_name else { continue };

            let type_def = self.graph.elements.values().find(|e| {
                e.name.as_deref() == Some(&tn)
                    && matches!(
                        e.kind,
                        ElementKind::AttributeDefinition | ElementKind::PartDefinition
                    )
            });
            let Some(td) = type_def else { continue };

            let mut map = std::collections::BTreeMap::new();
            for tc in self.graph.children_of(&td.id) {
                if tc.kind != ElementKind::AttributeUsage {
                    continue;
                }
                if let Some(tc_name) = &tc.name {
                    let val = tc
                        .get_prop("default")
                        .or_else(|| tc.get_prop("value"))
                        .and_then(|v| v.as_float());
                    if let Some(f) = val {
                        map.insert(tc_name.clone(), Value::Float(f));
                    }
                }
            }
            if !map.is_empty() {
                result.push((child_name, Value::Map(map)));
            }
        }
        result
    }

}
