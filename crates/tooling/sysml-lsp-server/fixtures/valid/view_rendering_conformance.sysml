// Sprint 12 conformance fixture: view/rendering and expose semantics
// Tests render_usage, expose_decl, and ViewRenderingMembership dispatch.

package ViewRenderingConformance {

    // Basic rendering definition
    rendering def StandardRendering {
    }

    // View definition with render usage (ViewRenderingMembership)
    view def SystemView {
        render :>> StandardRendering;
    }

    // Expose as standalone import (MembershipExpose with isImportAll=true)
    view def DetailView {
        expose SystemView;
    }

    // Expose namespace import (NamespaceExpose)
    view def FullView {
        expose ViewRenderingConformance::*;
    }
}
