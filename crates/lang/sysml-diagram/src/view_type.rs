//! Standard SysML view-family identifiers.
//!
//! This transport-neutral type identifies the standard-library `ViewDefinition`
//! family a [`crate::ViewRequest`] resolves to. It belongs to the diagram
//! contract, not to any renderer.

/// The standard SysML view family to generate.
///
/// The eight variants map directly to the standard-library `ViewDefinition`
/// set. Requirement notation is a filtered [`Self::General`] view; constraint
/// and binding notation uses [`Self::Interconnection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ViewType {
    General,
    Interconnection,
    StateTransition,
    ActionFlow,
    Browser,
    Sequence,
    Grid,
    Geometry,
}

impl ViewType {
    /// Map a caller-supplied transport string to a view family.
    ///
    /// This accepts canonical standard-library definition names, lowercase
    /// kind tokens, and short CLI aliases. It is only for request
    /// deserialisation; model resolution recognises canonical `*View`
    /// definitions through [`crate::view_request::resolve_view_kind`].
    pub fn from_request_str(s: &str) -> Option<Self> {
        match s {
            "GeneralView" | "general" => Some(Self::General),
            "InterconnectionView" | "interconnection" => Some(Self::Interconnection),
            "StateTransitionView" | "statetransition" | "state" => Some(Self::StateTransition),
            "ActionFlowView" | "actionflow" | "action" => Some(Self::ActionFlow),
            "BrowserView" | "browser" => Some(Self::Browser),
            "SequenceView" | "sequence" => Some(Self::Sequence),
            "GridView" | "grid" => Some(Self::Grid),
            "GeometryView" | "geometry" => Some(Self::Geometry),
            _ => None,
        }
    }
}
