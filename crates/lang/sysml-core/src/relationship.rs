//! Relationship types: `RelationshipKind` enum and `Relationship` struct.

use std::borrow::Cow;
use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_id::{CanonicalKey, ElementId};

use crate::meta::Value;

/// The kind of a relationship between elements.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum RelationshipKind {
    /// Ownership relationship (container -> contained).
    Owning,
    /// Type relationship (instance -> type).
    TypeOf,
    /// Satisfaction relationship (source = satisfying element -> target = requirement).
    Satisfy,
    /// Verification relationship (source = verification case -> target = requirement).
    Verify,
    /// Derivation relationship (source = derived requirement -> target = original).
    Derive,
    /// Refinement relationship (source = refining element -> target = refined).
    /// Discriminator: a Dependency annotated `ModelingMetadata::Refinement`.
    Refine,
    /// Traceability relationship (source = client -> target = supplier).
    Trace,
    /// Reference relationship.
    #[default]
    Reference,
    /// Specialization relationship (subtype -> supertype).
    Specialize,
    /// Redefinition relationship.
    Redefine,
    /// Subsetting relationship.
    Subsetting,
    /// Flow relationship.
    Flow,
    /// Transition relationship.
    Transition,
    /// Dependency relationship.
    Dependency,
    /// Import relationship (namespace import).
    Import,
    /// Allocate relationship.
    Allocate,
    /// Binding connection (equality constraint).
    Binding,
    /// Connection relationship.
    Connection,
    /// Perform relationship (action performed by part).
    Perform,
    /// Exhibit relationship (state exhibited by part).
    Exhibit,
    /// Include relationship (use case inclusion).
    Include,
    /// Succession relationship (temporal ordering).
    Succession,
    /// Composition relationship (filled diamond — composite feature membership).
    Composition,
    /// Annotation relationship (comment/metadata attachment).
    Annotation,
    /// Combined succession + flow relationship.
    SuccessionFlow,
    /// Message relationship (sequence diagram).
    Message,
    /// Non-composite feature membership (open diamond).
    FeatureMembership,
    /// Non-owning alias membership (unowned-membership).
    Membership,
    /// Flow adornment on an existing connection.
    FlowOnConnection,
    /// Interface connection (port-to-port typed connection edge).
    InterfaceConnection,
    /// Portion relationship (timeslice/snapshot of occurrence).
    Portion,
    /// Expose relationship (view-specific import).
    Expose,
    /// Frame relationship (requirement frames concern).
    Frame,
    /// Assert constraint edge (element asserts a constraint).
    Assert,
    /// Assume constraint edge (requirement assumes a constraint).
    Assume,
    /// Require constraint edge (requirement requires a constraint).
    Require,
    /// Distinguished parameter link (subject/actor/stakeholder).
    ParameterLink,
    /// Event occurrence edge (eventer to event occurrence).
    EventOccurrence,
}

impl RelationshipKind {
    /// Every `RelationshipKind` variant, in declaration order (iterable source of
    /// truth for the registration-manifest generator).
    pub const ALL: &'static [RelationshipKind] = &[
        RelationshipKind::Owning,
        RelationshipKind::TypeOf,
        RelationshipKind::Satisfy,
        RelationshipKind::Verify,
        RelationshipKind::Derive,
        RelationshipKind::Refine,
        RelationshipKind::Trace,
        RelationshipKind::Reference,
        RelationshipKind::Specialize,
        RelationshipKind::Redefine,
        RelationshipKind::Subsetting,
        RelationshipKind::Flow,
        RelationshipKind::Transition,
        RelationshipKind::Dependency,
        RelationshipKind::Import,
        RelationshipKind::Allocate,
        RelationshipKind::Binding,
        RelationshipKind::Connection,
        RelationshipKind::Perform,
        RelationshipKind::Exhibit,
        RelationshipKind::Include,
        RelationshipKind::Succession,
        RelationshipKind::Composition,
        RelationshipKind::Annotation,
        RelationshipKind::SuccessionFlow,
        RelationshipKind::Message,
        RelationshipKind::FeatureMembership,
        RelationshipKind::Membership,
        RelationshipKind::FlowOnConnection,
        RelationshipKind::InterfaceConnection,
        RelationshipKind::Portion,
        RelationshipKind::Expose,
        RelationshipKind::Frame,
        RelationshipKind::Assert,
        RelationshipKind::Assume,
        RelationshipKind::Require,
        RelationshipKind::ParameterLink,
        RelationshipKind::EventOccurrence,
    ];

    /// Returns an iterator over all relationship kinds.
    ///
    /// Keep this beside the enum definition so downstream crates do not
    /// maintain their own mirror lists for coverage tests or schemas.
    pub fn iter() -> impl Iterator<Item = RelationshipKind> {
        Self::ALL.iter().cloned()
    }

    /// Get the string representation of this kind.
    pub fn as_str(&self) -> &str {
        match self {
            RelationshipKind::Owning => "Owning",
            RelationshipKind::TypeOf => "TypeOf",
            RelationshipKind::Satisfy => "Satisfy",
            RelationshipKind::Verify => "Verify",
            RelationshipKind::Derive => "Derive",
            RelationshipKind::Refine => "Refine",
            RelationshipKind::Trace => "Trace",
            RelationshipKind::Reference => "Reference",
            RelationshipKind::Specialize => "Specialize",
            RelationshipKind::Redefine => "Redefine",
            RelationshipKind::Subsetting => "Subsetting",
            RelationshipKind::Flow => "Flow",
            RelationshipKind::Transition => "Transition",
            RelationshipKind::Dependency => "Dependency",
            RelationshipKind::Import => "Import",
            RelationshipKind::Allocate => "Allocate",
            RelationshipKind::Binding => "Binding",
            RelationshipKind::Connection => "Connection",
            RelationshipKind::Perform => "Perform",
            RelationshipKind::Exhibit => "Exhibit",
            RelationshipKind::Include => "Include",
            RelationshipKind::Succession => "Succession",
            RelationshipKind::Composition => "Composition",
            RelationshipKind::Annotation => "Annotation",
            RelationshipKind::SuccessionFlow => "SuccessionFlow",
            RelationshipKind::Message => "Message",
            RelationshipKind::FeatureMembership => "FeatureMembership",
            RelationshipKind::Membership => "Membership",
            RelationshipKind::FlowOnConnection => "FlowOnConnection",
            RelationshipKind::InterfaceConnection => "InterfaceConnection",
            RelationshipKind::Portion => "Portion",
            RelationshipKind::Expose => "Expose",
            RelationshipKind::Frame => "Frame",
            RelationshipKind::Assert => "Assert",
            RelationshipKind::Assume => "Assume",
            RelationshipKind::Require => "Require",
            RelationshipKind::ParameterLink => "ParameterLink",
            RelationshipKind::EventOccurrence => "EventOccurrence",
        }
    }

    /// The **wire** name — exactly how this variant serializes over JSON
    /// (`#[serde(rename_all = "camelCase")]` on the enum).
    ///
    /// This is the ONLY correct key for any map a JSON consumer will index by a
    /// serialized `RelationshipKind` (notably `DesignTokens::edge_styles`, which
    /// the renderer looks up as `edgeStyles[edge.kind.Relationship]`). Do NOT
    /// use `as_str()` / `format!("{self:?}")` for that — those are PascalCase and
    /// will silently miss every lookup, which is exactly the bug this method
    /// exists to prevent. `wire_name_matches_serde_representation` pins it to the
    /// real serde output for every variant so the two can never drift.
    pub fn wire_name(&self) -> &'static str {
        match self {
            RelationshipKind::Owning => "owning",
            RelationshipKind::TypeOf => "typeOf",
            RelationshipKind::Satisfy => "satisfy",
            RelationshipKind::Verify => "verify",
            RelationshipKind::Derive => "derive",
            RelationshipKind::Refine => "refine",
            RelationshipKind::Trace => "trace",
            RelationshipKind::Reference => "reference",
            RelationshipKind::Specialize => "specialize",
            RelationshipKind::Redefine => "redefine",
            RelationshipKind::Subsetting => "subsetting",
            RelationshipKind::Flow => "flow",
            RelationshipKind::Transition => "transition",
            RelationshipKind::Dependency => "dependency",
            RelationshipKind::Import => "import",
            RelationshipKind::Allocate => "allocate",
            RelationshipKind::Binding => "binding",
            RelationshipKind::Connection => "connection",
            RelationshipKind::Perform => "perform",
            RelationshipKind::Exhibit => "exhibit",
            RelationshipKind::Include => "include",
            RelationshipKind::Succession => "succession",
            RelationshipKind::Composition => "composition",
            RelationshipKind::Annotation => "annotation",
            RelationshipKind::SuccessionFlow => "successionFlow",
            RelationshipKind::Message => "message",
            RelationshipKind::FeatureMembership => "featureMembership",
            RelationshipKind::Membership => "membership",
            RelationshipKind::FlowOnConnection => "flowOnConnection",
            RelationshipKind::InterfaceConnection => "interfaceConnection",
            RelationshipKind::Portion => "portion",
            RelationshipKind::Expose => "expose",
            RelationshipKind::Frame => "frame",
            RelationshipKind::Assert => "assert",
            RelationshipKind::Assume => "assume",
            RelationshipKind::Require => "require",
            RelationshipKind::ParameterLink => "parameterLink",
            RelationshipKind::EventOccurrence => "eventOccurrence",
        }
    }
}

impl RelationshipKind {
    /// Returns the CSS line style for diagram edge rendering.
    ///
    /// Per SysML v2 graphical notation:
    /// - "dashed" for dependency-like relationships (satisfy, verify, import, etc.)
    /// - "dotted" for subsetting
    /// - "solid" for all others
    pub fn line_style(&self) -> &'static str {
        match self {
            RelationshipKind::Satisfy
            | RelationshipKind::Verify
            | RelationshipKind::Derive
            | RelationshipKind::Refine
            | RelationshipKind::Trace
            | RelationshipKind::Dependency
            | RelationshipKind::Import
            | RelationshipKind::Allocate
            | RelationshipKind::Perform
            | RelationshipKind::Exhibit
            | RelationshipKind::Include
            | RelationshipKind::Annotation
            | RelationshipKind::Membership
            | RelationshipKind::Expose
            | RelationshipKind::Assert
            | RelationshipKind::Assume
            | RelationshipKind::Require
            | RelationshipKind::EventOccurrence => "dashed",
            RelationshipKind::Subsetting => "dotted",
            _ => "solid",
        }
    }

    /// Returns CSS classes for diagram edge styling.
    ///
    /// Includes the lowercase kind name and the line style class.
    pub fn css_classes(&self) -> Vec<String> {
        let mut classes = vec![format!("{:?}", self).to_lowercase()];
        let style = self.line_style();
        if style != "solid" {
            classes.push(style.to_owned());
        }
        classes
    }

    /// Returns the PlantUML arrow notation for this relationship kind.
    pub fn plantuml_arrow(&self) -> &'static str {
        match self {
            RelationshipKind::Owning => "*--",
            RelationshipKind::TypeOf => "--|>",
            RelationshipKind::Specialize => "--|>",
            RelationshipKind::Redefine => "--|>",
            RelationshipKind::Subsetting => "..|>",
            RelationshipKind::Flow
            | RelationshipKind::Transition
            | RelationshipKind::Reference
            | RelationshipKind::Succession => "-->",
            RelationshipKind::Binding | RelationshipKind::Connection => "--",
            RelationshipKind::Satisfy
            | RelationshipKind::Verify
            | RelationshipKind::Derive
            | RelationshipKind::Refine
            | RelationshipKind::Trace
            | RelationshipKind::Dependency
            | RelationshipKind::Import
            | RelationshipKind::Allocate
            | RelationshipKind::Perform
            | RelationshipKind::Exhibit
            | RelationshipKind::Include
            | RelationshipKind::Annotation => "..>",
            RelationshipKind::Composition => "*--",
            RelationshipKind::SuccessionFlow
            | RelationshipKind::Message
            | RelationshipKind::FlowOnConnection
            | RelationshipKind::InterfaceConnection => "-->",
            RelationshipKind::FeatureMembership => "o--",
            RelationshipKind::Membership => "..>",
            RelationshipKind::Portion => "--|>",
            RelationshipKind::Expose
            | RelationshipKind::Assert
            | RelationshipKind::Assume
            | RelationshipKind::Require
            | RelationshipKind::EventOccurrence => "..>",
            RelationshipKind::Frame | RelationshipKind::ParameterLink => "--",
        }
    }

    /// Returns the single-character symbol for traceability matrix cells.
    pub fn matrix_symbol(&self) -> &'static str {
        match self {
            RelationshipKind::Satisfy => "S",
            RelationshipKind::Verify => "V",
            RelationshipKind::Allocate => "A",
            RelationshipKind::Derive => "D",
            RelationshipKind::Refine => "R",
            RelationshipKind::Trace => "T",
            RelationshipKind::Dependency => "\u{2192}", // →
            _ => "·",
        }
    }

    /// Returns true if this is a dependency-like relationship (dashed line in diagrams).
    pub fn is_dependency_like(&self) -> bool {
        self.line_style() == "dashed"
    }
}

impl std::fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A relationship between two elements.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Relationship {
    /// Unique identifier for this relationship.
    pub id: ElementId,
    /// The kind of this relationship.
    pub kind: RelationshipKind,
    /// The source element.
    pub source: ElementId,
    /// The target element.
    pub target: ElementId,
    /// Additional properties.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "BTreeMap::is_empty")
    )]
    pub props: BTreeMap<Cow<'static, str>, Value>,
}

impl Relationship {
    /// Feed this relationship's full content into a hasher.
    ///
    /// Counterpart of `Element::content_hash` — the one home for "what
    /// counts as this relationship's content". Source/target ids are
    /// load-bearing: without them a rewire between structurally
    /// symmetric siblings is invisible to the change-detection
    /// fingerprint (`ModelGraph::fingerprint`).
    pub fn content_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        self.id.hash(state);
        self.kind.hash(state);
        self.source.hash(state);
        self.target.hash(state);
        self.props.len().hash(state);
        for (key, value) in &self.props {
            key.hash(state);
            value.content_hash(state);
        }
    }

    /// Create a new relationship.
    ///
    /// canonical-key: synthetic-fresh-uuid — the reparse-unstable factory
    /// for elaboration / diagram-IR generators. Parser and elaboration
    /// mint sites that need reparse-stable ids should prefer
    /// [`Relationship::new_with_key`] (ADR-009).
    pub fn new(kind: RelationshipKind, source: ElementId, target: ElementId) -> Self {
        Relationship {
            id: ElementId::new_v4(),
            kind,
            source,
            target,
            props: BTreeMap::new(),
        }
    }

    /// Create a new relationship whose id is derived from a [`CanonicalKey`]
    /// (ADR-009).
    ///
    /// Use this in the elaboration / diagram-IR layer where reparse-stable
    /// identity matters. For synthetic / test relationships where reparse
    /// stability is not needed, use [`Relationship::new`], which mints a
    /// fresh UUID.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_core::{CanonicalKey, ElementId, Relationship, RelationshipKind};
    ///
    /// let project = CanonicalKey::root("my-project");
    /// let edge_key = CanonicalKey::for_anonymous(&project, "Specialize:source", 0);
    /// let src = ElementId::new_v4();
    /// let dst = ElementId::new_v4();
    /// let rel = Relationship::new_with_key(
    ///     RelationshipKind::Specialize,
    ///     src,
    ///     dst,
    ///     &edge_key,
    /// );
    /// assert_eq!(rel.id, edge_key.to_element_id());
    /// ```
    pub fn new_with_key(
        kind: RelationshipKind,
        source: ElementId,
        target: ElementId,
        canonical_key: &CanonicalKey,
    ) -> Self {
        Relationship {
            id: canonical_key.to_element_id(),
            kind,
            source,
            target,
            props: BTreeMap::new(),
        }
    }

    /// Create a relationship with a specific id.
    pub fn with_id(
        id: ElementId,
        kind: RelationshipKind,
        source: ElementId,
        target: ElementId,
    ) -> Self {
        Relationship {
            id,
            kind,
            source,
            target,
            props: BTreeMap::new(),
        }
    }

    /// Add a property.
    pub fn with_prop(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<Value>) -> Self {
        self.props.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_kind_all_const_is_complete() {
        // Bump this when a variant is added — and add it to ALL.
        assert_eq!(RelationshipKind::ALL.len(), 38);
    }

    #[test]
    fn relationship_kind_iter_matches_all_const() {
        let from_iter: Vec<RelationshipKind> = RelationshipKind::iter().collect();
        assert_eq!(from_iter.as_slice(), RelationshipKind::ALL);
    }

    /// `wire_name()` MUST equal the real serde output for every variant.
    ///
    /// This pins the contract that `DesignTokens::edge_styles` depends on. The
    /// renderer indexes that map with the serialized kind, so if the two ever
    /// drift the whole edge-style table silently stops resolving and every edge
    /// falls back to a default arrowhead — the exact bug this test prevents
    /// (the table was keyed PascalCase while the wire is camelCase, so it had
    /// never resolved at all).
    #[cfg(feature = "serde")]
    #[test]
    fn wire_name_matches_serde_representation() {
        for kind in RelationshipKind::ALL {
            let json = serde_json::to_value(kind).expect("RelationshipKind serializes");
            let serialized = json.as_str().expect("serializes to a bare JSON string");
            assert_eq!(
                kind.wire_name(),
                serialized,
                "wire_name() drifted from serde output for {kind:?}",
            );
        }
    }

    /// The wire name is deliberately NOT the Debug/PascalCase name — asserting
    /// the difference documents why a separate accessor exists at all.
    #[test]
    fn wire_name_is_not_the_debug_name() {
        assert_eq!(RelationshipKind::Connection.wire_name(), "connection");
        assert_eq!(RelationshipKind::Connection.as_str(), "Connection");
        assert_eq!(RelationshipKind::TypeOf.wire_name(), "typeOf");
        assert_eq!(RelationshipKind::FeatureMembership.wire_name(), "featureMembership");
    }
}
