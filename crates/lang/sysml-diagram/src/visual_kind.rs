//! Unified visual classification for SysML v2 diagram elements.
//!
//! This module consolidates the former `graphical_kind.rs` (type-safe enum) and
//! `classify.rs` (CSS/node-type helpers) into a single source of truth.
//!
//! Maps the 266-variant `ElementKind` (non-exhaustive, spec-generated) to a
//! compact `VisualKind` enum (~35 variants, exhaustive) that downstream code
//! can match without wildcards. Adding a new `VisualKind` variant forces
//! every `match` site to be updated — compile-time coverage enforcement.
//!
//! Also provides free-standing predicate functions (`is_port_kind`, `is_state_kind`,
//! etc.) used by view generators to filter elements.

use sysml_core::{Element, ElementKind, ModelGraph, RelationshipKind};

use crate::ir::types::NodeTag;

/// Backwards-compatible alias for code that still references `GraphicalKind`.
pub type GraphicalKind = VisualKind;

/// Visual shape category for a diagram element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shape {
    Rect,
    RoundedRect,
    Ellipse,
    Diamond,
    HBar,
    FilledCircle,
    BullseyeCircle,
    Pentagon,
    HourglassPentagon,
    NoteRect,
    DashedRect,
    CrossCircle,
}

/// The graphical category of a model element for diagram rendering.
///
/// This enum is intentionally **not** `#[non_exhaustive]` so that all `match`
/// expressions must be exhaustive — the compiler enforces full coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VisualKind {
    // Structural (rect / rounded rect)
    Package,
    Part,
    Item,
    Connection,

    // Behavioral (rounded rect variants)
    Action,
    State,
    Constraint,
    Calculation,

    // Requirements family
    Requirement,
    Concern,
    VerificationCase,

    // Cases
    UseCase,
    AnalysisCase,

    // Type-specific shapes
    Interface,
    Attribute,
    Enumeration,
    Allocation,
    Occurrence,
    Flow,
    View,
    Viewpoint,
    Port,
    Rendering,

    // Special shapes
    Comment,
    Metadata,
    Actor,

    // Control nodes
    InitialNode,
    FinalNode,
    DecisionNode,
    MergeNode,
    ForkNode,
    JoinNode,
    TerminateNode,
    SendAction,
    AcceptAction,

    // Sequence
    Lifeline,
    SqProxy,

    // Catch-all for non-diagrammatic elements (memberships, typing, etc.)
    Generic,
}

/// Compartment types that can appear inside diagram nodes.
///
/// Derived from the consolidated `compartment` production in `SysML-graphical-bnf.kgbnf`
/// (lines 2211-2226). Every spec compartment type is represented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CompartmentKind {
    // === Universal compartments ===
    /// Node header (stereotype + name labels)
    Header,
    /// `general-compartment` — textual/general content
    General,
    /// `features-compartment` — generic feature listing
    Features,
    /// `documentation-compartment`
    Documentation,

    // === Package compartments ===
    /// `packages-compartment` — nested packages
    Packages,
    /// `members-compartment` — generic members
    Members,
    /// `relationships-compartment` — relationship listing
    Relationships,

    // === Structural compartments ===
    /// `attributes-compartment`
    Attributes,
    /// `enums-compartment`
    Enums,
    /// `parts-compartment`
    Parts,
    /// `items-compartment`
    Items,
    /// `ports-compartment`
    Ports,
    /// `directed-features-compartment` — in/out/inout features
    DirectedFeatures,
    /// `interconnection-compartment` — internal block diagram
    Interconnection,
    /// `connections-compartment`
    Connections,
    /// `interfaces-compartment`
    Interfaces,
    /// `ends-compartment` — interface/connector ends
    Ends,

    // === Behavioral compartments ===
    /// `actions-compartment`
    Actions,
    /// `perform-actions-compartment`
    PerformActions,
    /// `performed-by-compartment` — lists performers of an action
    PerformedBy,
    /// `parameters-compartment`
    Parameters,
    /// `action-flow-compartment` — embedded action flow diagram
    ActionFlow,
    /// `states-compartment`
    States,
    /// `states-actions-compartment` — combined entry/do/exit
    StatesActions,
    /// `exhibit-states-compartment`
    ExhibitStates,
    /// `successions-compartment`
    Successions,
    /// `state-transition-compartment` — embedded state diagram
    StateTransition,
    /// State `entry` action sub-compartment
    Entry,
    /// State `do` action sub-compartment
    Do,
    /// State `exit` action sub-compartment
    Exit,
    /// `transitions` (edge listing inside state)
    Transitions,
    /// `flows-compartment`
    Flows,
    /// `sequence-compartment` — embedded sequence diagram
    Sequence,

    // === Calculation compartments ===
    /// `calcs-compartment`
    Calculations,
    /// `result-compartment`
    Results,

    // === Constraint compartments ===
    /// `constraints-compartment`
    Constraints,
    /// `assert-constraints-compartment`
    AssertConstraints,
    /// `assume-constraints-compartment`
    AssumeConstraints,
    /// `require-constraints-compartment`
    RequireConstraints,

    // === Requirement compartments ===
    /// `requirements-compartment`
    Requirements,
    /// `satisfy-requirements-compartment`
    SatisfyRequirements,
    /// `satisfies-compartment` — what this element satisfies
    Satisfies,
    /// `frames-compartment` — framed concerns
    Frames,
    /// `subject-compartment`
    Subject,
    /// `actors-compartment`
    Actors,
    /// `stakeholders-compartment`
    Stakeholders,
    /// `concerns-compartment`
    Concerns,

    // === Verification compartments ===
    /// `verifications-compartment`
    Verifications,
    /// `verifies-compartment` — what this element verifies
    Verifies,
    /// `verification-methods-compartment`
    VerificationMethods,

    // === Case compartments ===
    /// `objective-compartment`
    Objective,
    /// `analyses-compartment`
    Analyses,
    /// `use-cases-compartment`
    UseCases,
    /// `include-actions-compartment`
    IncludeActions,
    /// `includes-compartment` — include use cases
    Includes,

    // === Occurrence compartments ===
    /// `occurrences-compartment`
    Occurrences,
    /// `individuals-compartment`
    Individuals,
    /// `timeslices-compartment`
    Timeslices,
    /// `snapshots-compartment`
    Snapshots,

    // === Allocation compartments ===
    /// `allocations-compartment`
    Allocations,

    // === View compartments ===
    /// `views-compartment`
    Views,
    /// `viewpoints-compartment`
    Viewpoints,
    /// `exposes-compartment`
    Exposes,
    /// `filters-compartment`
    Filters,
    /// `rendering-compartment`
    Renderings,

    // === Variation compartments ===
    /// `variants-compartment`
    Variants,
    /// `variant-elementusages-compartment`
    VariantUsages,

    // === Annotation compartments ===
    /// Redefinition value assignments (`:>>` syntax)
    Redefinitions,
    /// `metadata` features/annotations
    Metadata,
}

/// Arrow head style for relationship edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ArrowHead {
    /// Filled triangle (generalization)
    Filled,
    /// Hollow triangle (specialization)
    Hollow,
    /// Open arrow (dependency, flow)
    Open,
    /// No arrowhead (binding)
    None,
}

/// Line style for relationship edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

impl VisualKind {
    /// Every `VisualKind` variant, in declaration order. The single iterable
    /// source of truth for code that must enumerate all visual kinds (e.g. the
    /// registration-manifest generator). Adding a variant requires adding it here
    /// — guarded by `all_const_is_complete` below.
    pub const ALL: &'static [VisualKind] = &[
        VisualKind::Package, VisualKind::Part, VisualKind::Item, VisualKind::Connection,
        VisualKind::Action, VisualKind::State, VisualKind::Constraint, VisualKind::Calculation,
        VisualKind::Requirement, VisualKind::Concern, VisualKind::VerificationCase,
        VisualKind::UseCase, VisualKind::AnalysisCase, VisualKind::Interface, VisualKind::Attribute,
        VisualKind::Enumeration, VisualKind::Allocation, VisualKind::Occurrence, VisualKind::Flow,
        VisualKind::View, VisualKind::Viewpoint, VisualKind::Port, VisualKind::Rendering,
        VisualKind::Comment, VisualKind::Metadata, VisualKind::Actor, VisualKind::InitialNode,
        VisualKind::FinalNode, VisualKind::DecisionNode, VisualKind::MergeNode, VisualKind::ForkNode,
        VisualKind::JoinNode, VisualKind::TerminateNode, VisualKind::SendAction,
        VisualKind::AcceptAction, VisualKind::Lifeline, VisualKind::SqProxy, VisualKind::Generic,
    ];

    /// Map an `ElementKind` to its graphical category.
    ///
    /// The `_ => Generic` arm is required because `ElementKind` is `#[non_exhaustive]`.
    /// All diagram-relevant kinds get explicit mappings — `Generic` is only for
    /// internal model plumbing (memberships, typing, etc.).
    pub fn from_element_kind(kind: &ElementKind) -> Self {
        match kind {
            // --- Package ---
            ElementKind::Package | ElementKind::LibraryPackage => VisualKind::Package,

            // --- Part ---
            ElementKind::PartDefinition | ElementKind::PartUsage => VisualKind::Part,

            // --- Item ---
            ElementKind::ItemDefinition | ElementKind::ItemUsage => VisualKind::Item,

            // --- Connection ---
            ElementKind::ConnectionDefinition
            | ElementKind::ConnectionUsage
            | ElementKind::ConnectorAsUsage
            | ElementKind::BindingConnectorAsUsage => VisualKind::Connection,

            // --- Action ---
            ElementKind::ActionDefinition
            | ElementKind::ActionUsage
            | ElementKind::PerformActionUsage
            | ElementKind::AssignmentActionUsage
            | ElementKind::ForLoopActionUsage
            | ElementKind::WhileLoopActionUsage
            | ElementKind::LoopActionUsage
            | ElementKind::IfActionUsage => VisualKind::Action,

            // --- State ---
            ElementKind::StateDefinition
            | ElementKind::StateUsage
            | ElementKind::ExhibitStateUsage
            | ElementKind::TransitionUsage => VisualKind::State,

            // --- Constraint ---
            ElementKind::ConstraintDefinition
            | ElementKind::ConstraintUsage
            | ElementKind::AssertConstraintUsage => VisualKind::Constraint,

            // --- Calculation ---
            ElementKind::CalculationDefinition | ElementKind::CalculationUsage => {
                VisualKind::Calculation
            }

            // --- Requirement ---
            ElementKind::RequirementDefinition
            | ElementKind::RequirementUsage
            | ElementKind::SatisfyRequirementUsage => VisualKind::Requirement,

            // --- Concern ---
            ElementKind::ConcernDefinition | ElementKind::ConcernUsage => VisualKind::Concern,

            // --- Verification Case ---
            ElementKind::VerificationCaseDefinition | ElementKind::VerificationCaseUsage => {
                VisualKind::VerificationCase
            }

            // --- Use Case ---
            ElementKind::UseCaseDefinition
            | ElementKind::UseCaseUsage
            | ElementKind::IncludeUseCaseUsage => VisualKind::UseCase,

            // --- Analysis Case ---
            ElementKind::AnalysisCaseDefinition | ElementKind::AnalysisCaseUsage => {
                VisualKind::AnalysisCase
            }

            // --- Case (generic) → UseCase visual ---
            ElementKind::CaseDefinition | ElementKind::CaseUsage => VisualKind::UseCase,

            // --- Interface ---
            ElementKind::InterfaceDefinition | ElementKind::InterfaceUsage => {
                VisualKind::Interface
            }

            // --- Attribute ---
            ElementKind::AttributeDefinition | ElementKind::AttributeUsage => {
                VisualKind::Attribute
            }

            // --- Enumeration ---
            ElementKind::EnumerationDefinition | ElementKind::EnumerationUsage => {
                VisualKind::Enumeration
            }

            // --- Allocation ---
            ElementKind::AllocationDefinition | ElementKind::AllocationUsage => {
                VisualKind::Allocation
            }

            // --- Occurrence ---
            ElementKind::OccurrenceDefinition
            | ElementKind::OccurrenceUsage
            | ElementKind::EventOccurrenceUsage => VisualKind::Occurrence,

            // --- Flow ---
            ElementKind::FlowDefinition
            | ElementKind::FlowUsage
            | ElementKind::SuccessionFlowUsage => VisualKind::Flow,

            // --- View ---
            ElementKind::ViewDefinition | ElementKind::ViewUsage => VisualKind::View,

            // --- Viewpoint ---
            ElementKind::ViewpointDefinition | ElementKind::ViewpointUsage => {
                VisualKind::Viewpoint
            }

            // --- Port ---
            ElementKind::PortDefinition
            | ElementKind::PortUsage
            | ElementKind::ConjugatedPortDefinition => VisualKind::Port,

            // --- Rendering ---
            ElementKind::RenderingDefinition | ElementKind::RenderingUsage => {
                VisualKind::Rendering
            }

            // --- Comment ---
            ElementKind::Comment | ElementKind::Documentation => VisualKind::Comment,

            // --- Metadata ---
            ElementKind::MetadataDefinition | ElementKind::MetadataUsage => VisualKind::Metadata,

            // --- Control nodes ---
            // ControlNode is the abstract KerML base — concrete nodes below
            ElementKind::ControlNode => VisualKind::Generic,
            ElementKind::ForkNode => VisualKind::ForkNode,
            ElementKind::JoinNode => VisualKind::JoinNode,
            ElementKind::DecisionNode => VisualKind::DecisionNode,
            ElementKind::MergeNode => VisualKind::MergeNode,
            ElementKind::TerminateActionUsage => VisualKind::TerminateNode,
            ElementKind::SendActionUsage => VisualKind::SendAction,
            ElementKind::AcceptActionUsage => VisualKind::AcceptAction,

            // Everything else (memberships, typing, internal plumbing)
            _ => VisualKind::Generic,
        }
    }

    /// Sprotty node type string (used as `type` field in SModel JSON).
    pub fn node_type(&self) -> &'static str {
        match self {
            Self::Package => "node:package",
            Self::Part => "node:block",
            Self::Item => "node:block",
            Self::Connection => "node:block",
            Self::Action => "node:action",
            Self::State => "node:state",
            Self::Constraint => "node:constraint",
            Self::Calculation => "node:action",
            Self::Requirement => "node:requirement",
            Self::Concern => "node:requirement",
            Self::VerificationCase => "node:requirement",
            Self::UseCase => "node:usecase",
            Self::AnalysisCase => "node:usecase",
            Self::Interface => "node:interface",
            Self::Attribute => "node:attribute",
            Self::Enumeration => "node:enumeration",
            Self::Allocation => "node:allocation",
            Self::Occurrence => "node:occurrence",
            Self::Flow => "node:block",
            Self::View => "node:view",
            Self::Viewpoint => "node:view",
            Self::Port => "port",
            Self::Rendering => "node:block",
            Self::Comment => "node:comment",
            Self::Metadata => "node:metadata",
            Self::Actor => "node:block",
            Self::InitialNode => "node:initialNode",
            Self::FinalNode => "node:finalNode",
            Self::DecisionNode => "node:decisionNode",
            Self::MergeNode => "node:mergeNode",
            Self::ForkNode => "node:forkNode",
            Self::JoinNode => "node:joinNode",
            Self::TerminateNode => "node:terminateNode",
            Self::SendAction => "node:sendAction",
            Self::AcceptAction => "node:acceptAction",
            Self::Lifeline => "node:lifeline",
            Self::SqProxy => "node:sqProxy",
            Self::Generic => "node:block",
        }
    }

    /// The canonical visual shape for this graphical kind.
    pub fn shape(&self) -> Shape {
        match self {
            Self::Package => Shape::Rect,
            Self::Part | Self::Item | Self::Connection => Shape::Rect,
            Self::Action | Self::Calculation => Shape::RoundedRect,
            Self::State => Shape::RoundedRect,
            Self::Constraint => Shape::RoundedRect,
            Self::Requirement | Self::Concern | Self::VerificationCase => Shape::Rect,
            Self::UseCase => Shape::Ellipse,
            Self::AnalysisCase => Shape::Rect,
            Self::Interface => Shape::Rect,
            Self::Attribute | Self::Enumeration => Shape::Rect,
            Self::Allocation | Self::Occurrence => Shape::Rect,
            Self::Flow => Shape::Rect,
            Self::View | Self::Viewpoint => Shape::Rect,
            Self::Port => Shape::Rect,
            Self::Rendering => Shape::Rect,
            Self::Comment => Shape::NoteRect,
            Self::Metadata => Shape::DashedRect,
            Self::Actor => Shape::Rect,
            Self::InitialNode => Shape::FilledCircle,
            Self::FinalNode => Shape::BullseyeCircle,
            Self::DecisionNode | Self::MergeNode => Shape::Diamond,
            Self::ForkNode | Self::JoinNode => Shape::HBar,
            Self::TerminateNode => Shape::CrossCircle,
            Self::SendAction => Shape::Pentagon,
            Self::AcceptAction => Shape::HourglassPentagon,
            Self::Lifeline => Shape::Rect,
            Self::SqProxy => Shape::FilledCircle,
            Self::Generic => Shape::Rect,
        }
    }

    /// CSS class name for element-type styling.
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Part => "part",
            Self::Item => "item",
            Self::Connection => "connection",
            Self::Action => "action",
            Self::State => "state",
            Self::Constraint => "constraint",
            Self::Calculation => "calc",
            Self::Requirement => "requirement",
            Self::Concern => "concern",
            Self::VerificationCase => "verification",
            Self::UseCase => "usecase",
            Self::AnalysisCase => "usecase",
            Self::Interface => "interface",
            Self::Attribute => "attribute",
            Self::Enumeration => "enumeration",
            Self::Allocation => "allocation",
            Self::Occurrence => "occurrence",
            Self::Flow => "flow",
            Self::View => "view",
            Self::Viewpoint => "viewpoint",
            Self::Port => "port",
            Self::Rendering => "rendering",
            Self::Comment => "comment",
            Self::Metadata => "metadata",
            Self::Actor => "actor",
            Self::InitialNode => "initial",
            Self::FinalNode => "final",
            Self::DecisionNode => "decision",
            Self::MergeNode => "merge",
            Self::ForkNode => "fork",
            Self::JoinNode => "join",
            Self::TerminateNode => "terminate",
            Self::SendAction => "send",
            Self::AcceptAction => "accept",
            Self::Lifeline => "lifeline",
            Self::SqProxy => "sq-proxy",
            Self::Generic => "generic",
        }
    }

    /// Whether this kind represents a definition (sharp corners) vs usage (rounded).
    /// Returns `None` for kinds that don't have the def/usage distinction.
    pub fn is_definition_or_usage(&self) -> Option<bool> {
        // This is more accurately determined from ElementKind string,
        // but we can indicate whether the distinction applies.
        match self {
            Self::Package
            | Self::Comment
            | Self::Metadata
            | Self::InitialNode
            | Self::FinalNode
            | Self::DecisionNode
            | Self::MergeNode
            | Self::ForkNode
            | Self::JoinNode
            | Self::TerminateNode
            | Self::SendAction
            | Self::AcceptAction
            | Self::Lifeline
            | Self::SqProxy
            | Self::Actor
            | Self::Generic => None,
            _ => Some(true), // Both defs and usages exist; actual value from ElementKind
        }
    }

    /// Compartments that every definition/usage node gets automatically.
    /// These are universal SysML features — attributes, documentation,
    /// redefinitions, metadata, etc. Only domain-specific compartments
    /// (e.g., Constraints on Requirements, Entry/Do/Exit on States) need
    /// to be listed per-VisualKind.
    const UNIVERSAL_COMPARTMENTS: &'static [CompartmentKind] = &[
        CompartmentKind::Header,
        CompartmentKind::Attributes,
        CompartmentKind::Features,
        CompartmentKind::Variants,
        CompartmentKind::Documentation,
        CompartmentKind::Redefinitions,
        CompartmentKind::Metadata,
    ];

    /// Domain-specific compartments for this graphical kind.
    ///
    /// These are ADDED to `UNIVERSAL_COMPARTMENTS`. Only list compartments
    /// that are specific to this element type per the SysML graphical BNF.
    fn domain_compartments(&self) -> &'static [CompartmentKind] {
        match self {
            Self::Package => &[
                CompartmentKind::General,
                CompartmentKind::Packages,
                CompartmentKind::Members,
                CompartmentKind::Relationships,
            ],
            Self::Part => &[
                CompartmentKind::DirectedFeatures,
                CompartmentKind::Parts,
                CompartmentKind::Ports,
                CompartmentKind::Actions,
                CompartmentKind::PerformActions,
                CompartmentKind::States,
                CompartmentKind::Constraints,
                CompartmentKind::Requirements,
                CompartmentKind::Connections,
                CompartmentKind::Interconnection,
                CompartmentKind::Flows,
                CompartmentKind::Allocations,
            ],
            Self::Item => &[
                CompartmentKind::Items,
                CompartmentKind::Ports,
            ],
            Self::Connection => &[
                CompartmentKind::Ports,
                CompartmentKind::Ends,
                CompartmentKind::Connections,
            ],
            Self::Action => &[
                CompartmentKind::Parameters,
                CompartmentKind::PerformedBy,
                CompartmentKind::Actions,
                CompartmentKind::ActionFlow,
                CompartmentKind::Constraints,
                CompartmentKind::Flows,
            ],
            Self::State => &[
                CompartmentKind::Entry,
                CompartmentKind::Do,
                CompartmentKind::Exit,
                CompartmentKind::StatesActions,
                CompartmentKind::States,
                CompartmentKind::ExhibitStates,
                CompartmentKind::Transitions,
                CompartmentKind::Successions,
                CompartmentKind::StateTransition,
            ],
            Self::Constraint => &[
                CompartmentKind::Parameters,
                CompartmentKind::Constraints,
                CompartmentKind::AssertConstraints,
                CompartmentKind::AssumeConstraints,
            ],
            Self::Calculation => &[
                CompartmentKind::Parameters,
                CompartmentKind::Calculations,
                CompartmentKind::Results,
                CompartmentKind::Constraints,
            ],
            Self::Requirement => &[
                CompartmentKind::Constraints,
                CompartmentKind::AssertConstraints,
                CompartmentKind::AssumeConstraints,
                CompartmentKind::RequireConstraints,
                CompartmentKind::Requirements,
                CompartmentKind::SatisfyRequirements,
                CompartmentKind::Satisfies,
                CompartmentKind::Subject,
                CompartmentKind::Actors,
                CompartmentKind::Stakeholders,
                CompartmentKind::Frames,
                CompartmentKind::Verifications,
            ],
            Self::Concern => &[
                CompartmentKind::Constraints,
                CompartmentKind::Requirements,
                CompartmentKind::Stakeholders,
                CompartmentKind::Frames,
            ],
            Self::VerificationCase => &[
                CompartmentKind::Objective,
                CompartmentKind::Subject,
                CompartmentKind::Actions,
                CompartmentKind::Verifications,
                CompartmentKind::Verifies,
                CompartmentKind::VerificationMethods,
            ],
            Self::UseCase => &[
                CompartmentKind::Objective,
                CompartmentKind::Subject,
                CompartmentKind::Actors,
                CompartmentKind::Actions,
                CompartmentKind::UseCases,
                CompartmentKind::IncludeActions,
                CompartmentKind::Includes,
            ],
            Self::AnalysisCase => &[
                CompartmentKind::Objective,
                CompartmentKind::Subject,
                CompartmentKind::Actors,
                CompartmentKind::Actions,
                CompartmentKind::Analyses,
                CompartmentKind::Results,
            ],
            Self::Interface => &[
                CompartmentKind::Ports,
                CompartmentKind::Ends,
                CompartmentKind::Flows,
                CompartmentKind::Interfaces,
            ],
            Self::Attribute => &[],
            Self::Enumeration => &[CompartmentKind::Enums],
            Self::Allocation => &[CompartmentKind::Allocations],
            Self::Occurrence => &[
                CompartmentKind::Parts,
                CompartmentKind::Occurrences,
                CompartmentKind::Individuals,
                CompartmentKind::Timeslices,
                CompartmentKind::Snapshots,
                CompartmentKind::Sequence,
            ],
            Self::Flow => &[CompartmentKind::Flows],
            Self::View => &[
                CompartmentKind::Views,
                CompartmentKind::Viewpoints,
                CompartmentKind::Exposes,
                CompartmentKind::Filters,
                CompartmentKind::Renderings,
            ],
            Self::Viewpoint => &[
                CompartmentKind::Concerns,
                CompartmentKind::Stakeholders,
            ],
            Self::Rendering => &[
                CompartmentKind::Renderings,
                CompartmentKind::Members,
            ],
            // Leaf/control nodes — no domain compartments
            _ => &[],
        }
    }

    /// Allowed compartment types for this graphical kind.
    ///
    /// Combines universal compartments (Header, Attributes, Documentation,
    /// Features, Variants, Redefinitions, Metadata) with domain-specific ones.
    /// Control nodes and special types override entirely.
    pub fn allowed_compartments(&self) -> Vec<CompartmentKind> {
        match self {
            // Special cases that don't get universal compartments
            Self::Comment => vec![CompartmentKind::Documentation],
            Self::Metadata => vec![CompartmentKind::Header, CompartmentKind::Attributes],
            Self::Actor => vec![CompartmentKind::Header],
            Self::Port => vec![
                CompartmentKind::Header,
                CompartmentKind::Attributes,
                CompartmentKind::Ports,
                CompartmentKind::Features,
            ],
            Self::InitialNode | Self::FinalNode | Self::DecisionNode
            | Self::MergeNode | Self::ForkNode | Self::JoinNode
            | Self::TerminateNode | Self::SqProxy => vec![],
            Self::SendAction | Self::AcceptAction | Self::Lifeline => {
                vec![CompartmentKind::Header]
            }
            Self::Generic => vec![CompartmentKind::Header, CompartmentKind::Members],
            // All other types: universal + domain-specific
            _ => {
                let domain = self.domain_compartments();
                let mut result = Vec::with_capacity(
                    Self::UNIVERSAL_COMPARTMENTS.len() + domain.len(),
                );
                result.extend_from_slice(Self::UNIVERSAL_COMPARTMENTS);
                result.extend_from_slice(domain);
                result
            }
        }
    }

    /// Given a child element's `VisualKind`, determine which compartment type
    /// it should be placed in within this (parent) node.
    ///
    /// Checks `allowed_compartments()` and their `allowed_child_kinds()` to find
    /// the most specific matching compartment. Falls back to `Members` if no
    /// specific compartment matches.
    pub fn compartment_for_child(&self, child_kind: VisualKind) -> CompartmentKind {
        for comp in self.allowed_compartments() {
            let allowed = comp.allowed_child_kinds();
            if !allowed.is_empty() && allowed.contains(&child_kind) {
                return comp;
            }
        }
        CompartmentKind::Members
    }

    /// Like `compartment_for_child`, but uses `ElementKind` for finer-grained
    /// routing of "shadowed" compartment types.
    ///
    /// For example, `PerformActionUsage` → `PerformActions` instead of `Actions`,
    /// `ExhibitStateUsage` → `ExhibitStates` instead of `States`, etc.
    pub fn compartment_for_element_kind(&self, element_kind: &ElementKind) -> CompartmentKind {
        // Check for specific ElementKind → CompartmentKind overrides first
        let specific = match element_kind {
            ElementKind::PerformActionUsage => Some(CompartmentKind::PerformActions),
            ElementKind::ExhibitStateUsage => Some(CompartmentKind::ExhibitStates),
            ElementKind::AssertConstraintUsage => Some(CompartmentKind::AssertConstraints),
            ElementKind::SatisfyRequirementUsage => Some(CompartmentKind::SatisfyRequirements),
            ElementKind::IncludeUseCaseUsage => Some(CompartmentKind::IncludeActions),
            ElementKind::TransitionUsage => Some(CompartmentKind::Transitions),
            _ => None,
        };
        if let Some(comp) = specific {
            // Only use the override if the parent actually allows this compartment
            if self.allowed_compartments().contains(&comp) {
                return comp;
            }
        }
        // Fall back to VisualKind-based routing
        self.compartment_for_child(VisualKind::from_element_kind(element_kind))
    }

    /// Like `compartment_for_element_kind`, but also checks element properties
    /// for finer-grained routing into property-based compartments.
    ///
    /// Routes based on:
    /// - `direction` property → `DirectedFeatures` (for attributes with in/out/inout)
    /// - `isEnd` property → `Ends` (for ports that are connection ends)
    /// - `isVariation` property → `Variants` (for variant children)
    /// - `isIndividual` property → `Individuals` (for individual occurrences)
    /// - `isPortion` + timeslice/snapshot → `Timeslices` / `Snapshots`
    pub fn compartment_for_element(&self, element: &Element) -> CompartmentKind {
        let allowed = self.allowed_compartments();

        // direction property → DirectedFeatures compartment
        if element.get_prop("direction").is_some()
            && allowed.contains(&CompartmentKind::DirectedFeatures)
        {
            return CompartmentKind::DirectedFeatures;
        }

        // isEnd property → Ends compartment
        if element
            .get_prop("isEnd")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && allowed.contains(&CompartmentKind::Ends)
        {
            return CompartmentKind::Ends;
        }

        // isVariation property → Variants compartment
        if element
            .get_prop("isVariation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && allowed.contains(&CompartmentKind::Variants)
        {
            return CompartmentKind::Variants;
        }

        // isIndividual property → Individuals compartment
        if element
            .get_prop("isIndividual")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && allowed.contains(&CompartmentKind::Individuals)
        {
            return CompartmentKind::Individuals;
        }

        // isPortion property → Timeslices or Snapshots compartment
        if element
            .get_prop("isPortion")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            // Distinguish timeslice vs snapshot by portionKind property
            let portion_kind = element
                .get_prop("portionKind")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if portion_kind == "snapshot" && allowed.contains(&CompartmentKind::Snapshots) {
                return CompartmentKind::Snapshots;
            }
            if allowed.contains(&CompartmentKind::Timeslices) {
                return CompartmentKind::Timeslices;
            }
        }

        // Fall back to ElementKind-based routing
        self.compartment_for_element_kind(&element.kind)
    }

    /// Like `compartment_for_element`, but with graph access for structural checks.
    ///
    /// Detects redefinitions (unnamed elements with Redefinition children) and
    /// metadata (MetadataUsage elements) that require graph traversal.
    pub fn compartment_for_element_with_graph(
        &self,
        element: &Element,
        graph: &sysml_core::ModelGraph,
    ) -> CompartmentKind {
        let allowed = self.allowed_compartments();

        // MetadataUsage → Metadata compartment
        if element.kind == sysml_core::ElementKind::MetadataUsage
            && allowed.contains(&CompartmentKind::Metadata)
        {
            return CompartmentKind::Metadata;
        }

        // Unnamed element with Redefinition child → Redefinitions compartment
        if element.name.is_none() && allowed.contains(&CompartmentKind::Redefinitions) {
            let has_redefinition = graph
                .children_of(&element.id)
                .any(|c| c.kind == sysml_core::ElementKind::Redefinition);
            if has_redefinition {
                return CompartmentKind::Redefinitions;
            }
        }

        // Fall through to property + kind routing
        self.compartment_for_element(element)
    }

    /// Whether this kind is a control node (no compartments, special shape).
    pub fn is_control_node(&self) -> bool {
        matches!(
            self,
            Self::InitialNode
                | Self::FinalNode
                | Self::DecisionNode
                | Self::MergeNode
                | Self::ForkNode
                | Self::JoinNode
                | Self::TerminateNode
        )
    }

    /// Whether this kind represents a port (rendered on node boundaries).
    pub fn is_port(&self) -> bool {
        *self == Self::Port
    }

    /// Whether this kind is a requirement family type.
    pub fn is_requirement_family(&self) -> bool {
        matches!(
            self,
            Self::Requirement | Self::Concern | Self::VerificationCase
        )
    }
}

impl CompartmentKind {
    /// Every `CompartmentKind` variant, in declaration order (the iterable source
    /// of truth for the registration-manifest generator).
    pub const ALL: &'static [CompartmentKind] = &[
        CompartmentKind::Header, CompartmentKind::General, CompartmentKind::Features,
        CompartmentKind::Documentation, CompartmentKind::Packages, CompartmentKind::Members,
        CompartmentKind::Relationships, CompartmentKind::Attributes, CompartmentKind::Enums,
        CompartmentKind::Parts, CompartmentKind::Items, CompartmentKind::Ports,
        CompartmentKind::DirectedFeatures, CompartmentKind::Interconnection,
        CompartmentKind::Connections, CompartmentKind::Interfaces, CompartmentKind::Ends,
        CompartmentKind::Actions, CompartmentKind::PerformActions, CompartmentKind::PerformedBy,
        CompartmentKind::Parameters, CompartmentKind::ActionFlow, CompartmentKind::States,
        CompartmentKind::StatesActions, CompartmentKind::ExhibitStates, CompartmentKind::Successions,
        CompartmentKind::StateTransition, CompartmentKind::Entry, CompartmentKind::Do,
        CompartmentKind::Exit, CompartmentKind::Transitions, CompartmentKind::Flows,
        CompartmentKind::Sequence, CompartmentKind::Calculations, CompartmentKind::Results,
        CompartmentKind::Constraints, CompartmentKind::AssertConstraints,
        CompartmentKind::AssumeConstraints, CompartmentKind::RequireConstraints,
        CompartmentKind::Requirements, CompartmentKind::SatisfyRequirements,
        CompartmentKind::Satisfies, CompartmentKind::Frames, CompartmentKind::Subject,
        CompartmentKind::Actors, CompartmentKind::Stakeholders, CompartmentKind::Concerns,
        CompartmentKind::Verifications, CompartmentKind::Verifies,
        CompartmentKind::VerificationMethods, CompartmentKind::Objective, CompartmentKind::Analyses,
        CompartmentKind::UseCases, CompartmentKind::IncludeActions, CompartmentKind::Includes,
        CompartmentKind::Occurrences, CompartmentKind::Individuals, CompartmentKind::Timeslices,
        CompartmentKind::Snapshots, CompartmentKind::Allocations, CompartmentKind::Views,
        CompartmentKind::Viewpoints, CompartmentKind::Exposes, CompartmentKind::Filters,
        CompartmentKind::Renderings, CompartmentKind::Variants, CompartmentKind::VariantUsages,
        CompartmentKind::Redefinitions, CompartmentKind::Metadata,
    ];

    /// Sprotty compartment type string.
    pub fn type_string(&self) -> &'static str {
        match self {
            // Universal
            Self::Header => "comp:header",
            Self::General => "comp:general",
            Self::Features => "comp:features",
            Self::Documentation => "comp:documentation",
            // Package
            Self::Packages => "comp:packages",
            Self::Members => "comp:members",
            Self::Relationships => "comp:relationships",
            // Structural
            Self::Attributes => "comp:attributes",
            Self::Enums => "comp:enums",
            Self::Parts => "comp:parts",
            Self::Items => "comp:items",
            Self::Ports => "comp:ports",
            Self::DirectedFeatures => "comp:directedFeatures",
            Self::Interconnection => "comp:interconnection",
            Self::Connections => "comp:connections",
            Self::Interfaces => "comp:interfaces",
            Self::Ends => "comp:ends",
            // Behavioral
            Self::Actions => "comp:actions",
            Self::PerformActions => "comp:performActions",
            Self::PerformedBy => "comp:performedBy",
            Self::Parameters => "comp:parameters",
            Self::ActionFlow => "comp:actionFlow",
            Self::States => "comp:states",
            Self::StatesActions => "comp:statesActions",
            Self::ExhibitStates => "comp:exhibitStates",
            Self::Successions => "comp:successions",
            Self::StateTransition => "comp:stateTransition",
            Self::Entry => "comp:entry",
            Self::Do => "comp:do",
            Self::Exit => "comp:exit",
            Self::Transitions => "comp:transitions",
            Self::Flows => "comp:flows",
            Self::Sequence => "comp:sequence",
            // Calculation
            Self::Calculations => "comp:calculations",
            Self::Results => "comp:results",
            // Constraint
            Self::Constraints => "comp:constraints",
            Self::AssertConstraints => "comp:assertConstraints",
            Self::AssumeConstraints => "comp:assumeConstraints",
            Self::RequireConstraints => "comp:requireConstraints",
            // Requirement
            Self::Requirements => "comp:requirements",
            Self::SatisfyRequirements => "comp:satisfyRequirements",
            Self::Satisfies => "comp:satisfies",
            Self::Frames => "comp:frames",
            Self::Subject => "comp:subject",
            Self::Actors => "comp:actors",
            Self::Stakeholders => "comp:stakeholders",
            Self::Concerns => "comp:concerns",
            // Verification
            Self::Verifications => "comp:verifications",
            Self::Verifies => "comp:verifies",
            Self::VerificationMethods => "comp:verificationMethods",
            // Case
            Self::Objective => "comp:objective",
            Self::Analyses => "comp:analyses",
            Self::UseCases => "comp:useCases",
            Self::IncludeActions => "comp:includeActions",
            Self::Includes => "comp:includes",
            // Occurrence
            Self::Occurrences => "comp:occurrences",
            Self::Individuals => "comp:individuals",
            Self::Timeslices => "comp:timeslices",
            Self::Snapshots => "comp:snapshots",
            // Allocation
            Self::Allocations => "comp:allocations",
            // View
            Self::Views => "comp:views",
            Self::Viewpoints => "comp:viewpoints",
            Self::Exposes => "comp:exposes",
            Self::Filters => "comp:filters",
            Self::Renderings => "comp:renderings",
            // Variation
            Self::Variants => "comp:variants",
            Self::VariantUsages => "comp:variantUsages",
            // Annotation
            Self::Redefinitions => "comp:redefinitions",
            Self::Metadata => "comp:metadata",
        }
    }

    /// What kinds of elements are expected inside this compartment.
    pub fn allowed_child_kinds(&self) -> &'static [VisualKind] {
        match self {
            // Labels only, no child elements
            Self::Header | Self::General => &[],
            // Structural
            Self::Attributes | Self::DirectedFeatures => &[VisualKind::Attribute],
            Self::Enums => &[VisualKind::Enumeration],
            Self::Parts => &[VisualKind::Part],
            Self::Items => &[VisualKind::Item],
            Self::Ports | Self::Ends => &[VisualKind::Port],
            Self::Connections | Self::Interconnection => &[VisualKind::Connection],
            Self::Interfaces => &[VisualKind::Interface],
            // Behavioral
            Self::Actions | Self::PerformActions | Self::IncludeActions => {
                &[VisualKind::Action]
            }
            Self::PerformedBy => &[], // Contains QualifiedName labels, not child elements
            Self::Parameters => &[VisualKind::Attribute],
            Self::ActionFlow => &[], // Contains sub-diagram (nodes + edges)
            Self::States | Self::ExhibitStates | Self::StatesActions => &[VisualKind::State],
            Self::Transitions | Self::Successions => &[], // Edges, not nodes
            Self::StateTransition => &[], // Contains sub-diagram (nodes + edges)
            Self::Entry | Self::Do | Self::Exit => &[VisualKind::Action],
            Self::Flows => &[VisualKind::Flow],
            Self::Sequence => &[], // Contains lifelines + messages
            // Calculation
            Self::Calculations => &[VisualKind::Calculation],
            Self::Results => &[VisualKind::Attribute],
            // Constraint
            Self::Constraints | Self::AssertConstraints | Self::AssumeConstraints
            | Self::RequireConstraints => &[VisualKind::Constraint],
            // Requirement
            Self::Requirements | Self::SatisfyRequirements => {
                &[VisualKind::Requirement, VisualKind::Concern]
            }
            Self::Satisfies | Self::Frames => &[], // Relationship references
            Self::Subject => &[VisualKind::Part],
            Self::Actors => &[VisualKind::Actor, VisualKind::Part],
            Self::Stakeholders => &[VisualKind::Part],
            Self::Concerns => &[VisualKind::Concern],
            // Verification
            Self::Verifications => &[VisualKind::VerificationCase],
            Self::Verifies | Self::VerificationMethods => &[], // Relationship references
            // Case
            Self::Objective => &[VisualKind::Requirement],
            Self::Analyses => &[VisualKind::AnalysisCase],
            Self::UseCases | Self::Includes => &[VisualKind::UseCase],
            // Occurrence
            Self::Occurrences => &[VisualKind::Occurrence],
            Self::Individuals | Self::Timeslices | Self::Snapshots => &[VisualKind::Occurrence],
            // Allocation
            Self::Allocations => &[VisualKind::Allocation],
            // View
            Self::Views => &[VisualKind::View],
            Self::Viewpoints => &[VisualKind::Viewpoint],
            Self::Exposes | Self::Filters => &[], // Expressions/references
            Self::Renderings => &[VisualKind::Rendering],
            // Variation
            Self::Variants | Self::VariantUsages => &[], // Any kind can be a variant
            // Other
            Self::Packages => &[VisualKind::Package],
            Self::Members | Self::Features | Self::Relationships => &[], // Generic — any kind
            Self::Documentation => &[VisualKind::Comment],
            Self::Redefinitions => &[], // Text-only (name = value lines)
            Self::Metadata => &[VisualKind::Metadata],
        }
    }
}

/// Edge rendering properties for a `RelationshipKind`.
pub struct EdgeStyle {
    pub arrowhead: ArrowHead,
    pub line_style: LineStyle,
    pub label: Option<&'static str>,
}

impl EdgeStyle {
    pub fn from_relationship_kind(kind: &sysml_core::RelationshipKind) -> Self {
        use sysml_core::RelationshipKind;
        match kind {
            RelationshipKind::Owning => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Solid,
                label: None,
            },
            // FeatureTyping ("defined by") — the graphical BNF `definition`
            // production (SysML-graphical-bnf.kgbnf, `type-relationship`) is the
            // image `definition.svg`: a SOLID line (`stroke-dasharray:none`) with
            // a HOLLOW triangle (`fill:none`) at the type end — identical in line
            // + head to `subclassification` (see `Specialize` below), NOT the
            // Open/Dashed dependency style it had (D-N8). FeatureTyping is
            // distinguished from Subclassification only by two small filled dots
            // at the type end (`definition.svg` path7/path9) — a distinct marker
            // deferred as D-N8b (needs an FE decoration, not an arrowhead reuse).
            // Bare-image production → no text label (see #70).
            RelationshipKind::TypeOf => EdgeStyle {
                arrowhead: ArrowHead::Hollow,
                line_style: LineStyle::Solid,
                label: None,
            },
            // R6 (OMG formal/26-03-02 §8.2.3.21/24, Tables 20/22): Satisfy and
            // Verify are DEDICATED relationships — SOLID line + open arrowhead +
            // «keyword», NOT the dashed generic-Dependency style. (The dashed
            // style was a SysML v1/UML holdover.)
            RelationshipKind::Satisfy => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: Some("«satisfy»"),
            },
            RelationshipKind::Verify => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: Some("«verify»"),
            },
            RelationshipKind::Derive => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«deriveReqt»"),
            },
            RelationshipKind::Refine => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«refine»"),
            },
            RelationshipKind::Trace => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«trace»"),
            },
            RelationshipKind::Reference => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Specialize => EdgeStyle {
                arrowhead: ArrowHead::Hollow,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Redefine => EdgeStyle {
                arrowhead: ArrowHead::Hollow,
                line_style: LineStyle::Solid,
                label: Some("«redefines»"),
            },
            RelationshipKind::Subsetting => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dotted,
                label: Some("«subsets»"),
            },
            RelationshipKind::Flow => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Transition => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Dependency => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: None,
            },
            RelationshipKind::Import => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«import»"),
            },
            RelationshipKind::Allocate => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«allocate»"),
            },
            RelationshipKind::Binding => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Connection => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Perform => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«perform»"),
            },
            RelationshipKind::Exhibit => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«exhibit»"),
            },
            RelationshipKind::Include => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«include»"),
            },
            RelationshipKind::Succession => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Composition => EdgeStyle {
                arrowhead: ArrowHead::Filled,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Annotation => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Dashed,
                label: None,
            },
            RelationshipKind::SuccessionFlow => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Message => EdgeStyle {
                arrowhead: ArrowHead::Filled,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::FeatureMembership => EdgeStyle {
                arrowhead: ArrowHead::Hollow,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Membership => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Dashed,
                label: None,
            },
            RelationshipKind::FlowOnConnection => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::InterfaceConnection => EdgeStyle {
                arrowhead: ArrowHead::None,
                line_style: LineStyle::Solid,
                label: None,
            },
            RelationshipKind::Portion => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«portion»"),
            },
            RelationshipKind::Expose => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«expose»"),
            },
            RelationshipKind::Frame => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«frame»"),
            },
            RelationshipKind::Assert => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«assert»"),
            },
            RelationshipKind::Assume => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«assume»"),
            },
            RelationshipKind::Require => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: Some("«require»"),
            },
            RelationshipKind::ParameterLink => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: None,
            },
            RelationshipKind::EventOccurrence => EdgeStyle {
                arrowhead: ArrowHead::Open,
                line_style: LineStyle::Dashed,
                label: None,
            },
        }
    }
}

// ── Functions consolidated from classify.rs ──────────────────────────────

pub(crate) fn is_membership_kind(kind: &ElementKind) -> bool {
    *kind == ElementKind::Membership || kind.is_subtype_of(ElementKind::Membership)
}

pub(crate) fn is_import_kind(kind: &ElementKind) -> bool {
    *kind == ElementKind::Import || kind.is_subtype_of(ElementKind::Import)
}

/// Elements that clutter the Browser containment tree without being meaningful
/// containment members: memberships, imports, relationships (Subclassification,
/// Specialization, FeatureTyping, …) and annotations (Documentation, Comment,
/// …). Relationships are edges and annotations are metadata — neither is a
/// structural child, so the browser tree hides them (they rendered as
/// "«unnamed Subclassification»" / "«unnamed Documentation»" noise rows).
pub(crate) fn is_browser_noise_kind(kind: &ElementKind) -> bool {
    is_membership_kind(kind)
        || is_import_kind(kind)
        || *kind == ElementKind::Relationship
        || kind.is_subtype_of(ElementKind::Relationship)
        || *kind == ElementKind::AnnotatingElement
        || kind.is_subtype_of(ElementKind::AnnotatingElement)
}

/// Whether an element is "effectively top-level" — either has no owner,
/// or is only nested inside Package/LibraryPackage containers.
pub(crate) fn is_effectively_top_level(element: &Element, graph: &ModelGraph) -> bool {
    let mut current_owner = element.owner.as_ref();
    while let Some(owner_id) = current_owner {
        if let Some(owner) = graph.get_element(owner_id) {
            if !matches!(
                owner.kind,
                ElementKind::Package | ElementKind::LibraryPackage
            ) {
                return false;
            }
            current_owner = owner.owner.as_ref();
        } else {
            return false;
        }
    }
    true
}

/// Resolve a usage element's type definition via `unresolved_type` prop
/// or `FeatureTyping` children.
pub(crate) fn find_type_definition<'a>(
    graph: &'a ModelGraph,
    usage: &Element,
) -> Option<&'a Element> {
    // Strategy 1: direct unresolved_type property
    if let Some(type_name) = usage.get_prop("unresolved_type").and_then(|v| v.as_str()) {
        if let Some(def) = find_definition_by_name(graph, type_name) {
            return Some(def);
        }
    }
    // Strategy 2: FeatureTyping children
    for child in graph.children_of(&usage.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                if let Some(def) = find_definition_by_name(graph, type_name) {
                    return Some(def);
                }
            }
        }
    }
    None
}

/// Find a definition element by name (any kind that is_definition()).
fn find_definition_by_name<'a>(graph: &'a ModelGraph, name: &str) -> Option<&'a Element> {
    graph
        .elements
        .values()
        .find(|e| e.kind.is_definition() && e.name.as_deref() == Some(name))
}

/// Public-to-crate lookup of a definition by name (any `is_definition()`
/// kind). Used by the view-kind supertype walk to follow a named
/// supertype to its own declaration and continue walking transitively.
pub(crate) fn definition_by_name<'a>(graph: &'a ModelGraph, name: &str) -> Option<&'a Element> {
    find_definition_by_name(graph, name)
}

/// Collect the **author-written** supertype / type names declared directly
/// on an element, regardless of resolution:
/// - `:` typing — a direct `unresolved_type` prop, or `FeatureTyping`
///   children carrying `unresolved_type` (a usage typed by a definition).
/// - `:>` specialization — `Subclassification` children carrying
///   `unresolved_superclassifier` (a definition specializing a definition).
///
/// Returns the raw names as written (possibly qualified). Used by the
/// view-kind resolver to walk the `:>` / `:` chain to a canonical standard
/// view definition. Resolution-independent on purpose: the canonical
/// `view def Interconnection :> InterconnectionView` aliases live in the
/// std-lib, which is not merged into the workspace graph, so we match on
/// the written name rather than a resolved element.
pub(crate) fn supertype_names(graph: &ModelGraph, element: &Element) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(t) = element.get_prop("unresolved_type").and_then(|v| v.as_str()) {
        names.push(t.to_owned());
    }
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(t) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                names.push(t.to_owned());
            }
        } else if child.kind == ElementKind::Subclassification
            || child.kind.is_subtype_of(ElementKind::Subclassification)
        {
            if let Some(t) = child
                .get_prop("unresolved_superclassifier")
                .and_then(|v| v.as_str())
            {
                names.push(t.to_owned());
            }
        }
    }
    names
}

pub(crate) fn is_requirement_relationship(kind: &RelationshipKind) -> bool {
    matches!(
        kind,
        RelationshipKind::Satisfy
            | RelationshipKind::Verify
            | RelationshipKind::Derive
            | RelationshipKind::Trace
    )
}

pub(crate) fn is_requirement_kind(kind: &ElementKind) -> bool {
    VisualKind::from_element_kind(kind).is_requirement_family()
}

pub(crate) fn is_state_kind(kind: &ElementKind) -> bool {
    matches!(VisualKind::from_element_kind(kind), VisualKind::State)
}

/// Whether an element kind should be shown as a child in the StateTransitionView.
pub(crate) fn is_state_transition_child_kind(kind: &ElementKind) -> bool {
    if *kind == ElementKind::TransitionUsage {
        return false;
    }
    if is_state_kind(kind) {
        return true;
    }
    if kind.is_control_node() {
        return true;
    }
    if matches!(
        kind,
        ElementKind::PerformActionUsage | ElementKind::ActionUsage
    ) {
        return true;
    }
    false
}

pub(crate) fn is_interconnection_kind(kind: &ElementKind) -> bool {
    matches!(
        VisualKind::from_element_kind(kind),
        VisualKind::Part
            | VisualKind::Port
            | VisualKind::Connection
            | VisualKind::Flow
            | VisualKind::Interface
            | VisualKind::Item
    )
}

pub(crate) fn is_part_kind(kind: &ElementKind) -> bool {
    matches!(VisualKind::from_element_kind(kind), VisualKind::Part)
}

pub(crate) fn is_port_kind(kind: &ElementKind) -> bool {
    VisualKind::from_element_kind(kind).is_port()
}

pub(crate) fn is_action_kind(kind: &ElementKind) -> bool {
    matches!(VisualKind::from_element_kind(kind), VisualKind::Action)
}

/// Context-aware graphical kind that considers ownership relationships.
pub(crate) fn effective_graphical_kind(element: &Element, graph: &ModelGraph) -> VisualKind {
    let base = VisualKind::from_element_kind(&element.kind);

    // Check for actor: PartUsage owned via ActorMembership
    if base == VisualKind::Part {
        if let Some(owner_id) = &element.owner {
            for rel in graph.incoming(&element.id) {
                if rel.kind == RelationshipKind::Owning {
                    if let Some(membership) = graph.get_element(&rel.source) {
                        if membership.kind == ElementKind::ActorMembership {
                            return VisualKind::Actor;
                        }
                    }
                }
            }
            if let Some(owner) = graph.get_element(owner_id) {
                if owner.kind == ElementKind::ActorMembership {
                    return VisualKind::Actor;
                }
            }
        }
    }

    base
}

/// Additional CSS classes derived from element properties.
/// Renderer-agnostic semantic tags derived from an element's properties
/// (replaces the former `property_css_classes`). The Sprotty adapter maps each
/// tag back to its CSS class; other renderers map it to their own styling.
pub(crate) fn property_tags(element: &Element) -> Vec<NodeTag> {
    let mut tags = Vec::new();

    if matches!(
        element.kind,
        ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
    ) {
        if element
            .get_prop("isAssume")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tags.push(NodeTag::AssumeConstraint);
        }
        if element
            .get_prop("isRequire")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tags.push(NodeTag::RequireConstraint);
        }
    }

    if matches!(element.kind, ElementKind::ConcernUsage)
        && element.get_prop("framedConcern").is_some() {
            tags.push(NodeTag::FrameConcern);
        }

    if matches!(element.kind, ElementKind::VerificationCaseUsage)
        && element.get_prop("verifiedRequirement").is_some() {
            tags.push(NodeTag::VerifyRequirement);
        }

    tags
}

/// Check if a port element has conjugated typing.
pub(crate) fn is_conjugated_port(element: &Element, graph: &ModelGraph) -> bool {
    if element
        .get_prop("isConjugated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::ConjugatedPortTyping {
            return true;
        }
    }
    false
}

/// Whether an element should be shown as a child in the GeneralView (BDD).
pub fn is_bdd_relevant(element: &Element) -> bool {
    let kind = &element.kind;

    if kind.is_relationship() && !kind.is_usage() && !kind.is_definition() {
        return false;
    }
    if kind.is_expression() && !kind.is_usage() && !kind.is_definition() {
        return false;
    }
    if matches!(
        kind,
        ElementKind::MultiplicityRange
            | ElementKind::Multiplicity
            | ElementKind::ResultExpressionMembership
            | ElementKind::ReturnParameterMembership
            | ElementKind::ParameterMembership
            | ElementKind::TextualRepresentation
            | ElementKind::MetadataUsage // rendered as compartment text, not nodes
    ) {
        return false;
    }
    if element.get_prop("stateSubactionKind").is_some() {
        return false;
    }
    // Unnamed connector-like usages (interface connect, bind, connect) are
    // structural relationships, not displayable parts. Suppress them in BDD —
    // they belong in IBD as edges, not in BDD as nodes/text.
    if element.name.is_none()
        && matches!(
            kind,
            ElementKind::InterfaceUsage
                | ElementKind::ConnectionUsage
                | ElementKind::BindingConnectorAsUsage
                | ElementKind::ConnectorAsUsage
        )
    {
        return false;
    }
    if element.name.is_none() {
        let gk = VisualKind::from_element_kind(kind);
        if gk == VisualKind::Generic {
            return false;
        }
        if gk.is_control_node() {
            return false;
        }
    }
    true
}

/// Map an ElementKind to its SysML keyword string.
pub fn element_keyword(kind: &ElementKind) -> String {
    if let Some(base_keyword) = kind.syntax_keyword() {
        if kind.is_definition() {
            format!("{} def", base_keyword)
        } else {
            base_keyword.to_owned()
        }
    } else {
        kind.display_name().to_owned()
    }
}

/// Map an ElementKind to its SModel node type string.
pub(crate) fn smodel_node_type(kind: &ElementKind) -> &'static str {
    VisualKind::from_element_kind(kind).node_type()
}

/// Map an ElementKind to CSS classes for Sprotty rendering.
pub(crate) fn element_css_classes(kind: &ElementKind) -> Vec<String> {
    let gk = VisualKind::from_element_kind(kind);
    let mut classes = vec![format!("{:?}", kind).to_lowercase()];
    if kind.is_definition() {
        classes.push("definition".to_owned());
    } else if kind.is_usage() {
        classes.push("usage".to_owned());
    }
    if gk != VisualKind::Generic {
        classes.push(gk.css_class().to_owned());
    }
    if matches!(kind, ElementKind::ReferenceUsage) {
        classes.push("reference".to_owned());
    }
    classes
}

/// Map a RelationshipKind to its SModel edge type string.
pub(crate) fn smodel_edge_type(kind: &RelationshipKind) -> &'static str {
    kind.edge_type()
}

/// Map a RelationshipKind to CSS classes for edge styling.
pub(crate) fn relationship_css_classes(kind: &RelationshipKind) -> Vec<String> {
    kind.css_classes()
}

/// Get port direction CSS class from element properties.
pub(crate) fn port_direction_css_class(element: &Element) -> Option<String> {
    element.get_prop("direction").and_then(|val| {
        let s = val.to_string();
        if s.contains("in") && s.contains("out") {
            Some("port-inout".to_owned())
        } else if s.contains("in") {
            Some("port-in".to_owned())
        } else if s.contains("out") {
            Some("port-out".to_owned())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_renderable_element_kinds_have_specific_graphical_kind() {
        let renderable = vec![
            // Structural
            ElementKind::Package,
            ElementKind::LibraryPackage,
            ElementKind::PartDefinition,
            ElementKind::PartUsage,
            ElementKind::ItemDefinition,
            ElementKind::ItemUsage,
            ElementKind::ConnectionDefinition,
            ElementKind::ConnectionUsage,
            // Behavioral
            ElementKind::ActionDefinition,
            ElementKind::ActionUsage,
            ElementKind::StateDefinition,
            ElementKind::StateUsage,
            ElementKind::ConstraintDefinition,
            ElementKind::ConstraintUsage,
            ElementKind::CalculationDefinition,
            ElementKind::CalculationUsage,
            // Requirements
            ElementKind::RequirementDefinition,
            ElementKind::RequirementUsage,
            ElementKind::ConcernDefinition,
            ElementKind::ConcernUsage,
            ElementKind::VerificationCaseDefinition,
            ElementKind::VerificationCaseUsage,
            // Cases
            ElementKind::UseCaseDefinition,
            ElementKind::UseCaseUsage,
            ElementKind::AnalysisCaseDefinition,
            ElementKind::AnalysisCaseUsage,
            ElementKind::CaseDefinition,
            ElementKind::CaseUsage,
            // Type-specific
            ElementKind::InterfaceDefinition,
            ElementKind::InterfaceUsage,
            ElementKind::AttributeDefinition,
            ElementKind::AttributeUsage,
            ElementKind::EnumerationDefinition,
            ElementKind::EnumerationUsage,
            ElementKind::AllocationDefinition,
            ElementKind::AllocationUsage,
            ElementKind::OccurrenceDefinition,
            ElementKind::OccurrenceUsage,
            ElementKind::FlowDefinition,
            ElementKind::FlowUsage,
            ElementKind::ViewDefinition,
            ElementKind::ViewUsage,
            ElementKind::ViewpointDefinition,
            ElementKind::ViewpointUsage,
            ElementKind::PortDefinition,
            ElementKind::PortUsage,
            ElementKind::RenderingDefinition,
            ElementKind::RenderingUsage,
            // Special
            ElementKind::Comment,
            ElementKind::Documentation,
            ElementKind::MetadataDefinition,
            ElementKind::MetadataUsage,
            // Control nodes
            ElementKind::ForkNode,
            ElementKind::JoinNode,
            ElementKind::DecisionNode,
            ElementKind::MergeNode,
            ElementKind::TerminateActionUsage,
            ElementKind::SendActionUsage,
            ElementKind::AcceptActionUsage,
            // Action subtypes
            ElementKind::PerformActionUsage,
            ElementKind::ExhibitStateUsage,
            ElementKind::IncludeUseCaseUsage,
            ElementKind::SatisfyRequirementUsage,
            ElementKind::AssertConstraintUsage,
            ElementKind::ForLoopActionUsage,
            ElementKind::WhileLoopActionUsage,
            ElementKind::IfActionUsage,
            ElementKind::AssignmentActionUsage,
            // Flow subtypes
            ElementKind::SuccessionFlowUsage,
            ElementKind::EventOccurrenceUsage,
            // Connector subtypes
            ElementKind::ConnectorAsUsage,
            ElementKind::BindingConnectorAsUsage,
            ElementKind::TransitionUsage,
            ElementKind::ConjugatedPortDefinition,
        ];

        for kind in renderable {
            let gk = VisualKind::from_element_kind(&kind);
            assert_ne!(
                gk,
                VisualKind::Generic,
                "ElementKind::{:?} maps to Generic — needs explicit VisualKind mapping",
                kind
            );
        }
    }

    #[test]
    fn non_renderable_kinds_map_to_generic() {
        let non_renderable = vec![
            ElementKind::Membership,
            ElementKind::FeatureTyping,
            ElementKind::Specialization,
            ElementKind::Redefinition,
            ElementKind::Subsetting,
            ElementKind::OwningMembership,
            ElementKind::FeatureMembership,
            ElementKind::Import,
        ];

        for kind in non_renderable {
            let gk = VisualKind::from_element_kind(&kind);
            assert_eq!(
                gk,
                VisualKind::Generic,
                "ElementKind::{:?} should map to Generic, got {:?}",
                kind,
                gk
            );
        }
    }

    #[test]
    fn node_type_strings_are_consistent() {
        // Verify key mappings match the existing classify.rs behavior
        assert_eq!(
            VisualKind::Part.node_type(),
            "node:block"
        );
        assert_eq!(
            VisualKind::Action.node_type(),
            "node:action"
        );
        assert_eq!(
            VisualKind::State.node_type(),
            "node:state"
        );
        assert_eq!(
            VisualKind::Requirement.node_type(),
            "node:requirement"
        );
        assert_eq!(
            VisualKind::UseCase.node_type(),
            "node:usecase"
        );
        assert_eq!(
            VisualKind::Port.node_type(),
            "port"
        );
        assert_eq!(
            VisualKind::Package.node_type(),
            "node:package"
        );
    }

    #[test]
    fn visual_kind_all_const_is_complete() {
        // Bump this when a variant is added — and add it to ALL.
        assert_eq!(VisualKind::ALL.len(), 38);
    }

    #[test]
    fn compartment_kind_all_const_is_complete() {
        // Bump this when a variant is added — and add it to ALL.
        assert_eq!(CompartmentKind::ALL.len(), 69);
    }

    #[test]
    fn every_graphical_kind_has_compartments_or_is_control_node() {
        for kind in VisualKind::ALL.iter().copied() {
            // `SqProxy` is a bare sequence-diagram proxy marker: no compartments,
            // not a control node. It is intentionally exempt from this invariant.
            if kind == VisualKind::SqProxy {
                continue;
            }
            // Every kind should either have compartments or be a control node
            let has_compartments = !kind.allowed_compartments().is_empty();
            let is_control = kind.is_control_node();
            assert!(
                has_compartments || is_control,
                "{:?} has no compartments and is not a control node",
                kind
            );
        }
    }

    #[test]
    fn edge_styles_cover_all_relationship_kinds() {
        use sysml_core::RelationshipKind;
        for kind in RelationshipKind::ALL.iter() {
            let _style = EdgeStyle::from_relationship_kind(kind);
            // Just verifying no panics and all are covered
        }
    }

    #[test]
    fn feature_typing_renders_as_solid_hollow_triangle() {
        use sysml_core::RelationshipKind;
        // D-N8: the graphical BNF `definition` production (`definition.svg`) is a
        // SOLID line + HOLLOW triangle at the type end — identical line/head to
        // `subclassification`, NOT the Open/Dashed dependency style. (The two are
        // distinguished only by FeatureTyping's dots, deferred as D-N8b.)
        let typing = EdgeStyle::from_relationship_kind(&RelationshipKind::TypeOf);
        assert_eq!(typing.arrowhead, ArrowHead::Hollow);
        assert_eq!(typing.line_style, LineStyle::Solid);
        assert_eq!(typing.label, None, "bare-image production carries no text label");

        // Sibling Subclassification shares the line + head (only the dots differ).
        let subclass = EdgeStyle::from_relationship_kind(&RelationshipKind::Specialize);
        assert_eq!(subclass.arrowhead, typing.arrowhead);
        assert_eq!(subclass.line_style, typing.line_style);
    }

    #[test]
    fn shadowed_compartments_route_correctly() {
        // PerformActionUsage → PerformActions (not Actions)
        let part = VisualKind::Part;
        assert_eq!(
            part.compartment_for_element_kind(&ElementKind::PerformActionUsage),
            CompartmentKind::PerformActions
        );
        // Regular ActionUsage → Actions
        assert_eq!(
            part.compartment_for_element_kind(&ElementKind::ActionUsage),
            CompartmentKind::Actions
        );

        // ExhibitStateUsage in Part → States (Part doesn't have ExhibitStates)
        assert_eq!(
            part.compartment_for_element_kind(&ElementKind::ExhibitStateUsage),
            CompartmentKind::States
        );
        // ExhibitStateUsage in State → ExhibitStates
        let state = VisualKind::State;
        assert_eq!(
            state.compartment_for_element_kind(&ElementKind::ExhibitStateUsage),
            CompartmentKind::ExhibitStates
        );

        // AssertConstraintUsage → AssertConstraints in Requirement parent
        let req = VisualKind::Requirement;
        assert_eq!(
            req.compartment_for_element_kind(&ElementKind::AssertConstraintUsage),
            CompartmentKind::AssertConstraints
        );

        // SatisfyRequirementUsage → SatisfyRequirements in Requirement parent
        assert_eq!(
            req.compartment_for_element_kind(&ElementKind::SatisfyRequirementUsage),
            CompartmentKind::SatisfyRequirements
        );

        // IncludeUseCaseUsage → IncludeActions in UseCase parent
        let uc = VisualKind::UseCase;
        assert_eq!(
            uc.compartment_for_element_kind(&ElementKind::IncludeUseCaseUsage),
            CompartmentKind::IncludeActions
        );
    }

    #[test]
    fn property_based_compartment_routing() {
        let part = VisualKind::Part;
        let iface = VisualKind::Interface;

        // AttributeUsage with direction → DirectedFeatures (Part has it)
        let mut attr = Element::new_with_kind(ElementKind::AttributeUsage).with_name("speed");
        attr.set_prop("direction", "in");
        assert_eq!(
            part.compartment_for_element(&attr),
            CompartmentKind::DirectedFeatures
        );

        // Same attribute without direction → Attributes
        let attr_no_dir = Element::new_with_kind(ElementKind::AttributeUsage).with_name("weight");
        assert_eq!(
            part.compartment_for_element(&attr_no_dir),
            CompartmentKind::Attributes
        );

        // Direction on Interface (no DirectedFeatures) → falls through to Attributes
        let mut attr_iface =
            Element::new_with_kind(ElementKind::AttributeUsage).with_name("rate");
        attr_iface.set_prop("direction", "out");
        assert_eq!(
            iface.compartment_for_element(&attr_iface),
            CompartmentKind::Attributes
        );

        // PortUsage with isEnd → Ends (in Interface parent)
        let mut end_port = Element::new_with_kind(ElementKind::PortUsage).with_name("src");
        end_port.set_prop("isEnd", true);
        assert_eq!(
            iface.compartment_for_element(&end_port),
            CompartmentKind::Ends
        );

        // PortUsage without isEnd → Ports (in Interface parent)
        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("data");
        assert_eq!(
            iface.compartment_for_element(&port),
            CompartmentKind::Ports
        );

        // Child with isVariation → Variants (in Part parent)
        let mut variant = Element::new_with_kind(ElementKind::PartUsage).with_name("optionA");
        variant.set_prop("isVariation", true);
        assert_eq!(
            part.compartment_for_element(&variant),
            CompartmentKind::Variants
        );

        // Without isVariation → Parts
        let normal = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
        assert_eq!(
            part.compartment_for_element(&normal),
            CompartmentKind::Parts
        );
    }

    // ─── Tests merged from classify.rs ──────────────────────────────────────

    #[test]
    fn node_type_mapping_covers_key_types() {
        let checks = vec![
            (ElementKind::Package, "node:package"),
            (ElementKind::PartDefinition, "node:block"),
            (ElementKind::PartUsage, "node:block"),
            (ElementKind::RequirementDefinition, "node:requirement"),
            (ElementKind::RequirementUsage, "node:requirement"),
            (ElementKind::StateDefinition, "node:state"),
            (ElementKind::StateUsage, "node:state"),
            (ElementKind::ActionDefinition, "node:action"),
            (ElementKind::ActionUsage, "node:action"),
            (ElementKind::ConstraintDefinition, "node:constraint"),
            (ElementKind::ConstraintUsage, "node:constraint"),
            (ElementKind::UseCaseDefinition, "node:usecase"),
            (ElementKind::UseCaseUsage, "node:usecase"),
            (ElementKind::InterfaceDefinition, "node:interface"),
            (ElementKind::InterfaceUsage, "node:interface"),
            (ElementKind::AttributeDefinition, "node:attribute"),
            (ElementKind::AttributeUsage, "node:attribute"),
            (ElementKind::EnumerationDefinition, "node:enumeration"),
            (ElementKind::EnumerationUsage, "node:enumeration"),
            (ElementKind::AllocationDefinition, "node:allocation"),
            (ElementKind::AllocationUsage, "node:allocation"),
            (ElementKind::OccurrenceDefinition, "node:occurrence"),
            (ElementKind::OccurrenceUsage, "node:occurrence"),
            (ElementKind::ViewDefinition, "node:view"),
            (ElementKind::ViewUsage, "node:view"),
            (ElementKind::ViewpointDefinition, "node:view"),
            (ElementKind::Comment, "node:comment"),
            (ElementKind::Documentation, "node:comment"),
            (ElementKind::MetadataDefinition, "node:metadata"),
            (ElementKind::SendActionUsage, "node:sendAction"),
            (ElementKind::AcceptActionUsage, "node:acceptAction"),
            (ElementKind::PortDefinition, "port"),
            (ElementKind::PortUsage, "port"),
            (ElementKind::CalculationDefinition, "node:action"),
            (ElementKind::CalculationUsage, "node:action"),
            (ElementKind::VerificationCaseDefinition, "node:requirement"),
            (ElementKind::VerificationCaseUsage, "node:requirement"),
            (ElementKind::AnalysisCaseDefinition, "node:usecase"),
            (ElementKind::AnalysisCaseUsage, "node:usecase"),
        ];

        for (kind, expected) in checks {
            let actual = smodel_node_type(&kind);
            assert_eq!(
                actual, expected,
                "ElementKind::{:?} should map to {:?}, got {:?}",
                kind, expected, actual
            );
        }
    }

    #[test]
    fn css_classes_include_element_type() {
        let classes = element_css_classes(&ElementKind::PartDefinition);
        assert!(classes.contains(&"definition".to_string()));
        assert!(classes.contains(&"part".to_string()));

        let classes = element_css_classes(&ElementKind::UseCaseUsage);
        assert!(classes.contains(&"usage".to_string()));
        assert!(classes.contains(&"usecase".to_string()));

        let classes = element_css_classes(&ElementKind::Comment);
        assert!(classes.contains(&"comment".to_string()));
    }

    #[test]
    fn element_keyword_uses_syntax_keyword() {
        assert_eq!(element_keyword(&ElementKind::PartDefinition), "part def");
        assert_eq!(element_keyword(&ElementKind::PartUsage), "part");
        assert_eq!(element_keyword(&ElementKind::ActionDefinition), "action def");
        assert_eq!(element_keyword(&ElementKind::ActionUsage), "action");
        assert_eq!(element_keyword(&ElementKind::Package), "package");
        assert_eq!(element_keyword(&ElementKind::EnumerationDefinition), "enum def");
        assert_eq!(element_keyword(&ElementKind::CalculationUsage), "calc");
        assert_eq!(element_keyword(&ElementKind::ViewDefinition), "view def");
    }

    #[test]
    fn smodel_edge_type_delegates_to_core() {
        assert_eq!(smodel_edge_type(&RelationshipKind::Satisfy), "edge:satisfy");
        assert_eq!(smodel_edge_type(&RelationshipKind::Flow), "edge:flow");
        assert_eq!(smodel_edge_type(&RelationshipKind::Succession), "edge:succession");
    }

    #[test]
    fn relationship_css_classes_delegates_to_core() {
        let classes = relationship_css_classes(&RelationshipKind::Satisfy);
        assert!(classes.contains(&"dashed".to_string()));

        let classes = relationship_css_classes(&RelationshipKind::Subsetting);
        assert!(classes.contains(&"dotted".to_string()));

        let classes = relationship_css_classes(&RelationshipKind::Specialize);
        assert!(!classes.contains(&"dashed".to_string()));
        assert!(!classes.contains(&"dotted".to_string()));
    }

    #[test]
    fn bdd_relevant_uses_generated_predicates() {
        // Relationships should be filtered
        let rel = Element::new_with_kind(ElementKind::Specialization);
        assert!(!is_bdd_relevant(&rel));

        // Expressions should be filtered
        let expr = Element::new_with_kind(ElementKind::LiteralInteger);
        assert!(!is_bdd_relevant(&expr));

        // Definitions should be relevant
        let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Foo");
        assert!(is_bdd_relevant(&def));
    }
}
