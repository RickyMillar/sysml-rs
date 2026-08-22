// 08 — Viewpoint satisfaction (the full chain).
//
// Asserts: `views.by_viewpoint(viewpointId)` walks the satisfy chain
// from a view usage back to its `viewpoint def`. Stakeholders →
// Viewpoints → Views → Renderings is the full spec ladder. The
// satisfaction claim sits on the view USAGE — the concrete view is what
// satisfies the stakeholder's concern; the definition carries the
// reusable recipe (supertype + filter).

package Demo {
    part def Reactor;
    part def CoolingSystem;
    part def Plant {
        part reactor : Reactor;
        part cooling : CoolingSystem;
    }
}

package Views {
    import Demo::*;
    // No wildcard import of StandardViewDefinitions here: that package
    // re-exports SysML::*, which would make the bare `kind` filter binding
    // ambiguous. Qualify the supertype instead.

    // Stakeholder
    part def SafetyEngineer;

    // Viewpoint — the stakeholder's concern, expressed as a requirement.
    viewpoint def MeltdownPreventionViewpoint {
        stakeholder reviewer : SafetyEngineer;
        doc /* The cooling system must always be visible alongside the
             * reactor it cools. This viewpoint is satisfied by any view
             * that exposes both elements together.
             */
    }

    // View definition — the recipe.
    view def ReactorCoolingView :> StandardViewDefinitions::InterconnectionView {
        filter kind == "PartUsage";
    }

    // Concrete instance — exposes content and satisfies the viewpoint.
    view reactorCooling : ReactorCoolingView {
        satisfy MeltdownPreventionViewpoint;
        expose Plant;
    }
}
