//! Library manager (post-P-RA4): now a no-op shell kept for layout
//! reasons. All library-state inquiries now go through
//! `sysml_service::SysmlService::readiness_for(_).library`, which is
//! populated by the `ProgressBus` lifecycle events emitted from
//! `workspace.rs`.

#![allow(clippy::module_name_repetitions)]

/// Placeholder so call-sites that historically held an
/// `Arc<LibraryManager>` continue to compile. Carries no state; the
/// canonical library lifecycle now lives on `SysmlService`.
pub struct LibraryManager;

impl LibraryManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LibraryManager {
    fn default() -> Self {
        Self::new()
    }
}
