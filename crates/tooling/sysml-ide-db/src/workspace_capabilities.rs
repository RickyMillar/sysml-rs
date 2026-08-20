//! Tracked query: workspace-level model-content capability flags.
//!
//! Powers S4.T3's `sysml.workspace.capabilities` service command, which
//! replaces the simulation-app's hand-written tree walk in
//! `editors/simulation-app/src/store/workspace.ts:178-321`. The frontend
//! used to merge a per-file tree walk and a per-file stats walk to
//! decide which UI paths to enable (state-machine panel, action panel,
//! requirements view, trade-study view, ODE editor, etc.). That walk is
//! now backend-owned: one pass over the elaborated workspace graph
//! produces every flag and name list the FE needs.
//!
//! Two variants mirror the rest of `analysis.rs`:
//!
//! - [`workspace_capabilities`] — workspace pfs only (no library
//!   overlay).
//! - [`workspace_capabilities_with_library`] — workspace + library.
//!
//! Keyed on the elaborated workspace; a file edit that doesn't change
//! the elaborated graph's element-kind histogram or names is a cache hit.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::{ElementKind, ModelGraph};

use crate::analysis::{elaborate_workspace, elaborate_workspace_with_library};
use crate::project_inputs::ProjectFileSet;
use crate::resolution::LibraryGraph;
use crate::Db;

/// Salsa-cached capability report for a workspace.
///
/// Wraps `Arc<WorkspaceCapabilitiesData>` with pointer-identity equality
/// — salsa returns the same Arc on cache hits within a revision. The
/// FE's `Capabilities` interface (`editors/simulation-app/src/store/workspace.ts:46`)
/// maps 1-to-1 onto these fields; T6 swaps the FE walk for this result.
#[derive(Clone, Debug)]
pub struct WorkspaceCapabilities(Arc<WorkspaceCapabilitiesData>);

#[derive(Debug)]
pub struct WorkspaceCapabilitiesData {
    pub has_state_machines: bool,
    pub has_action_flows: bool,
    pub has_ode_dynamics: bool,
    pub has_port_flows: bool,
    pub has_multiple_subsystems: bool,
    pub has_constraints: bool,
    pub has_requirements: bool,
    pub has_trade_studies: bool,
    pub state_machine_names: Vec<String>,
    pub action_flow_names: Vec<String>,
    pub trade_study_names: Vec<String>,
}

impl WorkspaceCapabilities {
    fn new(data: WorkspaceCapabilitiesData) -> Self {
        Self(Arc::new(data))
    }

    /// Borrow the inner data.
    pub fn data(&self) -> &WorkspaceCapabilitiesData {
        &self.0
    }
}

salsa_arc_wrapper!(identity, WorkspaceCapabilities, WorkspaceCapabilitiesData);

/// Walk an elaborated graph, classifying elements into the FE's
/// capability buckets. `file_count` powers the "multi-file workspace
/// with state machines ⇒ needs orchestrator mode" heuristic the FE
/// uses to gate cross-file SM execution.
///
/// The classification mirrors `detectCapabilitiesFromTree` +
/// `mergeCapabilitiesFromStats` from the FE: one pass produces names
/// (for state-machine / action / trade-study selectors) alongside the
/// boolean flags.
fn detect(graph: &ModelGraph, file_count: usize) -> WorkspaceCapabilitiesData {
    let mut has_state_machines = false;
    let mut has_action_flows = false;
    let mut has_ode_metadata = false;
    let mut has_port_flows = false;
    let mut has_constraints = false;
    let mut has_requirements = false;
    let mut has_trade_studies = false;

    let mut state_def_count = 0usize;
    let mut state_machine_names: Vec<String> = Vec::new();
    let mut action_flow_names: Vec<String> = Vec::new();
    let mut trade_study_names: Vec<String> = Vec::new();

    for element in graph.elements.values() {
        match element.kind {
            ElementKind::StateDefinition => {
                has_state_machines = true;
                state_def_count += 1;
                if let Some(name) = &element.name {
                    state_machine_names.push(name.clone());
                }
            }
            ElementKind::ExhibitStateUsage => {
                has_state_machines = true;
            }
            ElementKind::StateUsage => {
                // FE doesn't flip the flag on bare StateUsage — only
                // Definition / Exhibit — so neither do we.
            }
            ElementKind::ActionDefinition => {
                has_action_flows = true;
                if let Some(name) = &element.name {
                    action_flow_names.push(name.clone());
                }
            }
            ElementKind::ActionUsage => {
                has_action_flows = true;
            }
            ElementKind::ConstraintUsage
            | ElementKind::ConstraintDefinition
            | ElementKind::AssertConstraintUsage => {
                has_constraints = true;
            }
            ElementKind::RequirementDefinition | ElementKind::RequirementUsage => {
                has_requirements = true;
            }
            ElementKind::AnalysisCaseDefinition | ElementKind::AnalysisCaseUsage => {
                has_trade_studies = true;
                if let Some(name) = &element.name {
                    trade_study_names.push(name.clone());
                }
            }
            ElementKind::PortUsage | ElementKind::PortDefinition => {
                has_port_flows = true;
            }
            // The FE walker also checks `kind === 'FlowConnectionUsage'`
            // (`store/workspace.ts:218`) but that string never matches a
            // current ElementKind name — the canonical SysML v2 variant
            // is `FlowUsage`. The FE branch is dead today; treating it
            // as a deliberate signal anyway would expand the panel's
            // visibility unilaterally, which is a UX call we shouldn't
            // bundle into the T6 swap.
            ElementKind::MetadataUsage => {
                if element.name.as_deref() == Some("ToolExecution") {
                    has_ode_metadata = true;
                }
            }
            _ => {}
        }
    }

    // ODE dynamics aggregation:
    //   - explicit ToolExecution metadata on a state machine, OR
    //   - multi-file workspace with state machines (orchestrator mode).
    // The second rule mirrors the FE's
    // `if (statsCache.size > 1 && smDefCount > 0)` guard.
    let has_ode_dynamics =
        (has_ode_metadata && state_def_count > 0) || (file_count > 1 && state_def_count > 0);

    // FE's "multiple subsystems" = SM count + (any action flow ? 1 : 0).
    let subsystem_count =
        if has_state_machines { state_machine_names.len() } else { 0 } + usize::from(has_action_flows);
    let has_multiple_subsystems = subsystem_count > 1;

    WorkspaceCapabilitiesData {
        has_state_machines,
        has_action_flows,
        has_ode_dynamics,
        has_port_flows,
        has_multiple_subsystems,
        has_constraints,
        has_requirements,
        has_trade_studies,
        state_machine_names,
        action_flow_names,
        trade_study_names,
    }
}

/// Workspace capability report (no library overlay).
///
/// Depends on: `elaborate_workspace` + `ProjectFileSet.files()`.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub(crate) fn workspace_capabilities(db: &dyn Db, pfs: ProjectFileSet) -> WorkspaceCapabilities {
    let elaborated = elaborate_workspace(db, pfs);
    let file_count = pfs.files(db).len();
    WorkspaceCapabilities::new(detect(elaborated.graph(), file_count))
}

/// Workspace capability report with the standard library merged in.
///
/// Stdlib elements skew the kind histogram (~13k extra elements) but the
/// FE-equivalent behaviour treats library-derived state machines /
/// constraints / etc. as part of the workspace. Aligns with how
/// `workspace_descendants_with_library` and friends scope.
///
/// Depends on: `elaborate_workspace_with_library` + `ProjectFileSet.files()`
/// + `LibraryGraph`.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub(crate) fn workspace_capabilities_with_library(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: LibraryGraph,
) -> WorkspaceCapabilities {
    let elaborated = elaborate_workspace_with_library(db, pfs, library);
    let file_count = pfs.files(db).len();
    WorkspaceCapabilities::new(detect(elaborated.graph(), file_count))
}

/// Best-shape dispatcher — Some(lib) routes to ..._with_library, None to bare workspace.
#[tracing::instrument(level = "debug", skip(db, library))]
#[salsa::tracked]
pub fn workspace_capabilities_best(
    db: &dyn Db,
    pfs: ProjectFileSet,
    library: Option<LibraryGraph>,
) -> WorkspaceCapabilities {
    match library {
        Some(lib) => workspace_capabilities_with_library(db, pfs, lib),
        None => workspace_capabilities(db, pfs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::AnalysisHost;
    use crate::project_inputs::ProjectFileSet;
    use sysml_project::ProjectHandle;

    /// Spin up a host with `sources` registered under a single user
    /// project (pid 100), and return the host together with its
    /// `ProjectFileSet`.
    fn host_with_workspace(sources: &[(&str, &str)]) -> (AnalysisHost, ProjectFileSet) {
        let mut host = AnalysisHost::new();
        let pid = ProjectHandle(100);
        let project = sysml_project::Project {
            id: pid,
            info: sysml_project::ProjectInfo {
                name: "test".to_owned(),
                description: None,
                version: "1.0.0".to_owned(),
                topic: Vec::new(),
                usage: Vec::new(),
            },
            meta: None,
            root: sysml_project::ProjectRoot::InMemory,
        };
        host.load_project(project);

        let mut source_files = Vec::with_capacity(sources.len());
        for (uri, text) in sources {
            host.set_file_content_in_project(uri, (*text).to_owned(), pid);
            let fid = host.file_id(uri).expect("file id");
            source_files.push(host.source_file(fid).expect("source file"));
        }
        let pfs = ProjectFileSet::new(
            host.db(),
            pid.0,
            Arc::new(source_files),
            crate::project_inputs::PROJECT_KIND_DISCOVERED,
        );
        host.add_project_file_set(pfs);
        (host, pfs)
    }

    #[test]
    fn workspace_capabilities_best_dispatches_without_library() {
        let src = "package P { state def Tank; }";
        let (host, pfs) = host_with_workspace(&[("test.sysml", src)]);
        let analysis = host.analysis();
        let direct = workspace_capabilities(analysis.db(), pfs);
        let best = workspace_capabilities_best(analysis.db(), pfs, None);
        assert_eq!(direct.data().has_state_machines, best.data().has_state_machines);
        assert_eq!(direct.data().state_machine_names, best.data().state_machine_names);
    }

    #[test]
    fn empty_workspace_has_no_flags() {
        let (host, pfs) = host_with_workspace(&[("test.sysml", "")]);
        let analysis = host.analysis();
        let caps = workspace_capabilities(analysis.db(), pfs);
        let d = caps.data();
        assert!(!d.has_state_machines);
        assert!(!d.has_action_flows);
        assert!(!d.has_constraints);
        assert!(!d.has_requirements);
        assert!(!d.has_trade_studies);
        assert!(!d.has_port_flows);
        assert!(d.state_machine_names.is_empty());
    }

    #[test]
    fn detects_state_machine_definition() {
        let src = "package P { state def Tank; }";
        let (host, pfs) = host_with_workspace(&[("test.sysml", src)]);
        let analysis = host.analysis();
        let caps = workspace_capabilities(analysis.db(), pfs);
        let d = caps.data();
        assert!(
            d.has_state_machines,
            "state def should flip has_state_machines"
        );
        assert!(
            d.state_machine_names.iter().any(|n| n == "Tank"),
            "Tank should appear in state_machine_names; got {:?}",
            d.state_machine_names
        );
    }

    #[test]
    fn detects_constraints_and_requirements() {
        let src = r#"
            package P {
                requirement def R1;
                constraint def C1 {
                    1 > 0
                }
            }
        "#;
        let (host, pfs) = host_with_workspace(&[("test.sysml", src)]);
        let analysis = host.analysis();
        let caps = workspace_capabilities(analysis.db(), pfs);
        let d = caps.data();
        assert!(
            d.has_constraints,
            "constraint def should flip has_constraints"
        );
        assert!(
            d.has_requirements,
            "requirement def should flip has_requirements"
        );
    }

    #[test]
    fn multi_file_with_sm_implies_ode_dynamics() {
        // Two files, both with state defs → orchestrator mode signal.
        let (host, pfs) = host_with_workspace(&[
            ("a.sysml", "package A { state def S1; }"),
            ("b.sysml", "package B { state def S2; }"),
        ]);
        let analysis = host.analysis();
        let caps = workspace_capabilities(analysis.db(), pfs);
        let d = caps.data();
        assert!(d.has_state_machines);
        assert!(
            d.has_ode_dynamics,
            "multi-file workspace with state defs should imply has_ode_dynamics"
        );
        assert!(
            d.has_multiple_subsystems,
            "two named state machines should imply has_multiple_subsystems"
        );
    }

    #[test]
    fn cache_hit_returns_same_arc() {
        let (host, pfs) = host_with_workspace(&[("test.sysml", "package P { state def S; }")]);
        let analysis = host.analysis();
        let a = workspace_capabilities(analysis.db(), pfs);
        let b = workspace_capabilities(analysis.db(), pfs);
        assert!(
            Arc::ptr_eq(&a.0, &b.0),
            "salsa should memoize the capabilities report"
        );
    }
}
