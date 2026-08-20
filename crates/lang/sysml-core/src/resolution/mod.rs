//! Name resolution for SysML v2 model graphs.
//!
//! This module implements name resolution following the SysML v2/KerML scoping rules.
//! Resolution follows this precedence order:
//!
//! 1. **OWNED**: Local owned memberships of the namespace
//! 2. **INHERITED**: Members via Specialization chain (for Types)
//! 3. **IMPORTED**: Members from Import statements
//! 4. **PARENT**: Walk up to parent namespace
//! 5. **GLOBAL**: Root packages
//! 6. **LIBRARY**: Standard library package members (implicit)
//!
//! ## Submodules
//!
//! - [`scoping`]: Scoping strategies (owning, global, relative, etc.)
//! - [`scope_table`]: Cached scope information per namespace
//! - [`context`]: Resolution context with cycle detection and caching
//! - [`pass1`]: Pass 1 handlers (type relationships: Specialization, FeatureTyping, Subclassification)
//! - [`pass2`]: Pass 2 handlers (feature relationships: Subsetting, Redefinition, etc.)
//! - [`driver`]: Resolution driver orchestrating the two-pass algorithm
//!
//! ## Resolution Tracing
//!
//! Enable detailed resolution tracing by compiling with the `resolution-tracing` feature:
//!
//! ```bash
//! cargo build --features resolution-tracing
//! ```

mod scope_table;
pub mod scoping;
pub use scope_table::ScopeTable;
mod context;
pub use context::{InheritanceIndex, ResolutionContext};
mod driver;
mod pass1;
mod pass2;
mod pass_refs;

pub use driver::{
    resolve_references, resolve_references_excluding, resolve_references_excluding_pure,
    resolve_references_pure, resolve_references_with_fallback_pure, resolve_with_library,
    resolve_with_library_pure,
};

// Resolution tracing macros - enabled with the `resolution-tracing` feature
#[cfg(feature = "resolution-tracing")]
macro_rules! res_trace {
    ($($arg:tt)*) => {
        tracing::trace!($($arg)*);
    };
}

#[cfg(not(feature = "resolution-tracing"))]
macro_rules! res_trace {
    ($($arg:tt)*) => {};
}

// Allow the macro to be used in submodules
pub(crate) use res_trace;

// Re-export FxHashSet for callers of resolve_references_excluding
pub use rustc_hash::FxHashSet;

use std::borrow::Cow;

use sysml_id::ElementId;
use sysml_span::Diagnostics;

use crate::ModelGraph;

/// Property keys for unresolved references (as stored by parser).
pub mod unresolved_props {
    /// Unresolved supertype in Specialization.
    pub const GENERAL: &str = "unresolved_general";
    /// Unresolved type in FeatureTyping.
    pub const TYPE: &str = "unresolved_type";
    /// Unresolved subsetted feature in Subsetting.
    pub const SUBSETTED_FEATURE: &str = "unresolved_subsettedFeature";
    /// Unresolved subsetting feature in a STANDALONE Subsetting (`subset X subsets Y;`).
    /// The owned `:>` form sets `subsettingFeature` directly to the owning feature;
    /// only the namespace-member form leaves it to be resolved by name (G08e).
    pub const SUBSETTING_FEATURE: &str = "unresolved_subsettingFeature";
    /// Unresolved redefined feature in Redefinition.
    pub const REDEFINED_FEATURE: &str = "unresolved_redefinedFeature";
    /// Unresolved referenced feature in ReferenceSubsetting.
    pub const REFERENCED_FEATURE: &str = "unresolved_referencedFeature";
    /// Unresolved sources in Dependency.
    pub const SOURCES: &str = "unresolved_sources";
    /// Unresolved targets in Dependency.
    pub const TARGETS: &str = "unresolved_targets";
    /// Unresolved value expression.
    pub const VALUE: &str = "unresolved_value";

    // === Phase B: Additional cross-references ===

    /// Unresolved superclassifier in Subclassification.
    pub const SUPERCLASSIFIER: &str = "unresolved_superclassifier";
    /// Unresolved conjugated type in Conjugation.
    pub const CONJUGATED_TYPE: &str = "unresolved_conjugatedType";
    /// Unresolved original type in Conjugation.
    pub const ORIGINAL_TYPE: &str = "unresolved_originalType";
    /// Unresolved featuring type in TypeFeaturing.
    pub const FEATURING_TYPE: &str = "unresolved_featuringType";
    /// Unresolved disjoining type in Disjoining.
    pub const DISJOINING_TYPE: &str = "unresolved_disjoiningType";
    /// Unresolved unioning type in Unioning.
    pub const UNIONING_TYPE: &str = "unresolved_unioningType";
    /// Unresolved intersecting type in Intersecting.
    pub const INTERSECTING_TYPE: &str = "unresolved_intersectingType";
    /// Unresolved differencing type in Differencing.
    pub const DIFFERENCING_TYPE: &str = "unresolved_differencingType";
    /// Unresolved inverting feature in FeatureInverting.
    pub const INVERTING_FEATURE: &str = "unresolved_invertingFeature";
    /// Unresolved crossed feature in FeatureChaining.
    pub const CROSSED_FEATURE: &str = "unresolved_crossedFeature";
    /// Unresolved annotated element in Annotation.
    pub const ANNOTATED_ELEMENT: &str = "unresolved_annotatedElement";
    /// Unresolved member element in Membership.
    pub const MEMBER_ELEMENT: &str = "unresolved_memberElement";
    /// Unresolved client in Dependency.
    pub const CLIENT: &str = "unresolved_client";
    /// Unresolved supplier in Dependency.
    pub const SUPPLIER: &str = "unresolved_supplier";
    /// Unresolved conjugated port definition in ConjugatedPortDefinition.
    pub const CONJUGATED_PORT_DEFINITION: &str = "unresolved_conjugatedPortDefinition";
}

/// Property keys for Import elements (as stored by parser).
pub mod import_props {
    /// The qualified name of the imported reference.
    pub const IMPORTED_REFERENCE: &str = "importedReference";
    /// Whether this is a namespace import (::*).
    pub const IS_NAMESPACE: &str = "isNamespace";
    /// Whether this is a recursive import (::**).
    pub const IS_RECURSIVE: &str = "isRecursive";
    /// Whether 'all' keyword was used.
    pub const IMPORTS_ALL: &str = "importsAll";
    /// Whether all memberships are imported regardless of declared visibility
    /// (`import all`/`expose`). Default false → only public members.
    pub const IS_IMPORT_ALL: &str = "isImportAll";
    /// The Import's own visibility (re-export gate). Default `private`.
    pub const VISIBILITY: &str = "visibility";
}

/// Static map of primitive type aliases to their canonical names.
///
/// These are common shorthand aliases used in SysML that should resolve
/// to their canonical library types.
fn primitive_type_alias(name: &str) -> Option<&'static str> {
    match name {
        "float" => Some("Real"),
        "int" => Some("Integer"),
        _ => None,
    }
}

/// Well-known standard library package names.
///
/// These are the package names defined in the SysML v2 standard library.
/// Use these with `ModelGraph::register_library_package()` to enable
/// automatic resolution of library types.
pub mod library_packages {
    // === KerML Kernel Libraries ===
    /// Base types: Anything, DataValue, things, dataValues.
    pub const BASE: &str = "Base";
    /// Core element links.
    pub const LINKS: &str = "Links";
    /// Occurrence types.
    pub const OCCURRENCES: &str = "Occurrences";
    /// Object types.
    pub const OBJECTS: &str = "Objects";
    /// Performance types.
    pub const PERFORMANCES: &str = "Performances";
    /// Transfer types.
    pub const TRANSFERS: &str = "Transfers";
    /// Control performances.
    pub const CONTROL_PERFORMANCES: &str = "ControlPerformances";
    /// Transition performances.
    pub const TRANSITION_PERFORMANCES: &str = "TransitionPerformances";
    /// State performances.
    pub const STATE_PERFORMANCES: &str = "StatePerformances";
    /// Triggers.
    pub const TRIGGERS: &str = "Triggers";
    /// Scalar values: Boolean, String, Integer, Real, Complex, etc.
    pub const SCALAR_VALUES: &str = "ScalarValues";
    /// Vector values.
    pub const VECTOR_VALUES: &str = "VectorValues";
    /// Collections.
    pub const COLLECTIONS: &str = "Collections";
    /// Clocks.
    pub const CLOCKS: &str = "Clocks";
    /// Spatial frames.
    pub const SPATIAL_FRAMES: &str = "SpatialFrames";
    /// Observation.
    pub const OBSERVATION: &str = "Observation";
    /// Metaobjects.
    pub const METAOBJECTS: &str = "Metaobjects";
    /// KerML top-level library.
    pub const KERML: &str = "KerML";

    // === SysML Systems Libraries ===
    /// SysML top-level library.
    pub const SYSML: &str = "SysML";
    /// Items library.
    pub const ITEMS: &str = "Items";
    /// Parts library.
    pub const PARTS: &str = "Parts";
    /// Ports library.
    pub const PORTS: &str = "Ports";
    /// Actions library.
    pub const ACTIONS: &str = "Actions";
    /// States library.
    pub const STATES: &str = "States";
    /// Connections library.
    pub const CONNECTIONS: &str = "Connections";
    /// Interfaces library.
    pub const INTERFACES: &str = "Interfaces";
    /// Allocations library.
    pub const ALLOCATIONS: &str = "Allocations";
    /// Flows library.
    pub const FLOWS: &str = "Flows";
    /// Attributes library.
    pub const ATTRIBUTES: &str = "Attributes";
    /// Calculations library.
    pub const CALCULATIONS: &str = "Calculations";
    /// Constraints library.
    pub const CONSTRAINTS: &str = "Constraints";
    /// Requirements library.
    pub const REQUIREMENTS: &str = "Requirements";
    /// Cases library.
    pub const CASES: &str = "Cases";
    /// Analysis cases library.
    pub const ANALYSIS_CASES: &str = "AnalysisCases";
    /// Verification cases library.
    pub const VERIFICATION_CASES: &str = "VerificationCases";
    /// Use cases library.
    pub const USE_CASES: &str = "UseCases";
    /// Views library.
    pub const VIEWS: &str = "Views";
    /// Metadata library.
    pub const METADATA: &str = "Metadata";

    /// All KerML kernel library package names.
    pub const KERML_PACKAGES: &[&str] = &[
        BASE,
        LINKS,
        OCCURRENCES,
        OBJECTS,
        PERFORMANCES,
        TRANSFERS,
        CONTROL_PERFORMANCES,
        TRANSITION_PERFORMANCES,
        STATE_PERFORMANCES,
        TRIGGERS,
        SCALAR_VALUES,
        VECTOR_VALUES,
        COLLECTIONS,
        CLOCKS,
        SPATIAL_FRAMES,
        OBSERVATION,
        METAOBJECTS,
        KERML,
    ];

    /// All SysML systems library package names.
    pub const SYSML_PACKAGES: &[&str] = &[
        SYSML,
        ITEMS,
        PARTS,
        PORTS,
        ACTIONS,
        STATES,
        CONNECTIONS,
        INTERFACES,
        ALLOCATIONS,
        FLOWS,
        ATTRIBUTES,
        CALCULATIONS,
        CONSTRAINTS,
        REQUIREMENTS,
        CASES,
        ANALYSIS_CASES,
        VERIFICATION_CASES,
        USE_CASES,
        VIEWS,
        METADATA,
    ];

    /// All standard library package names (KerML + SysML).
    pub const ALL_PACKAGES: &[&str] = &[
        // KerML
        BASE,
        LINKS,
        OCCURRENCES,
        OBJECTS,
        PERFORMANCES,
        TRANSFERS,
        CONTROL_PERFORMANCES,
        TRANSITION_PERFORMANCES,
        STATE_PERFORMANCES,
        TRIGGERS,
        SCALAR_VALUES,
        VECTOR_VALUES,
        COLLECTIONS,
        CLOCKS,
        SPATIAL_FRAMES,
        OBSERVATION,
        METAOBJECTS,
        KERML,
        // SysML
        SYSML,
        ITEMS,
        PARTS,
        PORTS,
        ACTIONS,
        STATES,
        CONNECTIONS,
        INTERFACES,
        ALLOCATIONS,
        FLOWS,
        ATTRIBUTES,
        CALCULATIONS,
        CONSTRAINTS,
        REQUIREMENTS,
        CASES,
        ANALYSIS_CASES,
        VERIFICATION_CASES,
        USE_CASES,
        VIEWS,
        METADATA,
    ];
}

/// Property keys for resolved references.
pub mod resolved_props {
    /// Resolved supertype in Specialization.
    pub const GENERAL: &str = "general";
    /// Resolved type in FeatureTyping.
    pub const TYPE: &str = "type";
    /// Resolved subsetted feature in Subsetting.
    pub const SUBSETTED_FEATURE: &str = "subsettedFeature";
    /// Resolved subsetting feature in a standalone Subsetting (G08e).
    pub const SUBSETTING_FEATURE: &str = "subsettingFeature";
    /// Resolved redefined feature in Redefinition.
    pub const REDEFINED_FEATURE: &str = "redefinedFeature";
    /// Resolved referenced feature in ReferenceSubsetting.
    pub const REFERENCED_FEATURE: &str = "referencedFeature";
    /// Resolved sources in Dependency.
    pub const SOURCES: &str = "source";
    /// Resolved targets in Dependency.
    pub const TARGETS: &str = "target";
    /// Resolved feature reference in a `FeatureReferenceExpression`
    /// (`Value::Ref` to the referenced feature). Written by the
    /// reference-resolution pass; read by the semantic-token emitter to
    /// colour the reference by its resolved target's kind.
    pub const FEATURE_REFERENCE: &str = "featureReference";
    /// Resolved transition source state (`Value::Ref`). ADDITIVE — the
    /// existing `source` string prop is left untouched for existing readers.
    pub const TRANSITION_SOURCE: &str = "resolvedTransitionSource";
    /// Resolved transition target state (`Value::Ref`). ADDITIVE — the
    /// existing `target` string prop is left untouched for existing readers.
    pub const TRANSITION_TARGET: &str = "resolvedTransitionTarget";
    /// Resolved objective requirement (`Value::Ref`). ADDITIVE — the existing
    /// `objective` string prop is left untouched for existing readers.
    pub const OBJECTIVE: &str = "resolvedObjective";
    /// Resolved subject target on a `SubjectMembership` (`Value::Ref`).
    /// ADDITIVE — distinct from the `subject` prop `tag_subjects` writes on the
    /// requirement/case; this one lives on the membership so the semantic-token
    /// emitter can colour the `subject <name>` reference site. Also covers
    /// verification/use/analysis cases, which `tag_subjects` does not.
    pub const SUBJECT: &str = "resolvedSubject";
    /// Resolved target feature of an `AssignmentActionUsage` (`Value::Ref`).
    /// ADDITIVE — the existing `target`/`targetFeature` string props are left
    /// untouched. Lets the semantic-token emitter colour the `<target> = …`
    /// assignment reference site by the resolved feature's kind.
    pub const ASSIGNMENT_TARGET: &str = "resolvedAssignmentTarget";
    /// Resolved terminated occurrence of a `TerminateActionUsage` (`Value::Ref`).
    /// ADDITIVE — the parser's `unresolved_target` string prop is left
    /// untouched. Points at the feature the `terminate <ref>;` argument names
    /// (spec: the referent of the `NodeParameterMember`'s `FeatureBinding`
    /// expression, SysML.xtext `TerminateNode`/`NodeParameterMember`/
    /// `FeatureBinding`; vocab `terminatedOccurrenceArgument`,
    /// SysML-vocab.ttl).
    pub const TERMINATED_OCCURRENCE: &str = "resolvedTerminatedOccurrence";

    // === Phase B: Additional cross-references ===

    /// Resolved superclassifier in Subclassification.
    pub const SUPERCLASSIFIER: &str = "superclassifier";
    /// Resolved conjugated type in Conjugation.
    pub const CONJUGATED_TYPE: &str = "conjugatedType";
    /// Resolved original type in Conjugation.
    pub const ORIGINAL_TYPE: &str = "originalType";
    /// Resolved featuring type in TypeFeaturing.
    pub const FEATURING_TYPE: &str = "featuringType";
    /// Resolved disjoining type in Disjoining.
    pub const DISJOINING_TYPE: &str = "disjoiningType";
    /// Resolved unioning type in Unioning.
    pub const UNIONING_TYPE: &str = "unioningType";
    /// Resolved intersecting type in Intersecting.
    pub const INTERSECTING_TYPE: &str = "intersectingType";
    /// Resolved differencing type in Differencing.
    pub const DIFFERENCING_TYPE: &str = "differencingType";
    /// Resolved inverting feature in FeatureInverting.
    pub const INVERTING_FEATURE: &str = "invertingFeature";
    /// Resolved crossed feature in FeatureChaining.
    pub const CROSSED_FEATURE: &str = "crossedFeature";
    /// Resolved annotated element in Annotation.
    pub const ANNOTATED_ELEMENT: &str = "annotatedElement";
    /// Resolved member element in Membership.
    pub const MEMBER_ELEMENT: &str = "memberElement";
    /// Resolved client in Dependency.
    pub const CLIENT: &str = "client";
    /// Resolved supplier in Dependency.
    pub const SUPPLIER: &str = "supplier";
    /// Resolved conjugated port definition in ConjugatedPortDefinition.
    pub const CONJUGATED_PORT_DEFINITION: &str = "conjugatedPortDefinition";
}

/// Result of resolving references in a model graph.
#[derive(Debug, Default)]
pub struct ResolutionResult {
    /// Number of references successfully resolved.
    pub resolved_count: usize,
    /// Number of references that could not be resolved.
    pub unresolved_count: usize,
    /// Diagnostics collected during resolution.
    pub diagnostics: Diagnostics,
}

impl ResolutionResult {
    /// Create a new empty resolution result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if all references were resolved.
    pub fn is_complete(&self) -> bool {
        self.unresolved_count == 0
    }

    /// Check if there were any errors.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.has_errors()
    }
}

/// A single resolution update: set a property on an element to a resolved value.
///
/// This is the output of the pure resolution functions (`resolve_references_pure`,
/// `resolve_references_excluding_pure`). Updates are collected without mutating the
/// graph, allowing callers (e.g., salsa incremental computation) to inspect or
/// transform updates before applying them.
#[derive(Debug, Clone)]
pub struct ResolutionUpdate {
    /// The element to update.
    pub element_id: ElementId,
    /// The property name to set (e.g., "general", "type").
    pub property_name: Cow<'static, str>,
    /// The resolved target element ID.
    pub resolved_value: ElementId,
}

/// Apply a batch of resolution updates to a mutable model graph.
///
/// Each update sets a property on the target element to a `Value::Ref` pointing
/// at the resolved element. Elements that no longer exist in the graph are silently
/// skipped.
pub fn apply_resolution_updates(graph: &mut ModelGraph, updates: &[ResolutionUpdate]) {
    for update in updates {
        if let Some(element) = graph.elements.get_mut(&update.element_id) {
            element.set_prop(
                update.property_name.clone(),
                crate::Value::Ref(update.resolved_value.clone()),
            );
        }
    }
}

/// Extension trait for ModelGraph to provide resolution methods.
impl ModelGraph {
    /// Create a resolution context for this graph.
    pub fn resolution_context(&self) -> ResolutionContext<'_> {
        ResolutionContext::new(self)
    }

    /// Resolve a name within a namespace using a fresh context.
    ///
    /// This is a convenience method for simple resolution.
    /// For multiple resolutions, create a `ResolutionContext` instead.
    pub fn resolve_name_in(&self, namespace_id: &ElementId, name: &str) -> Option<ElementId> {
        let mut ctx = self.resolution_context();
        ctx.resolve_name(namespace_id, name)
    }

    /// Resolve a qualified name from root.
    ///
    /// This is a convenience method for simple resolution.
    pub fn resolve_qualified(&self, qname: &str) -> Option<ElementId> {
        let mut ctx = self.resolution_context();
        ctx.resolve_qualified_name_global(qname)
    }
}

#[cfg(test)]
mod tests;
