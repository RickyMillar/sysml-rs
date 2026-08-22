// 07 — Rendering binding.
//
// Asserts: a `render` clause in a view definition body binds the view to
// a rendering via a ViewRenderingMembership. The spec gives two forms:
// reference an existing rendering usage by name (`render someRendering;`)
// or declare a rendering usage inline from a rendering definition
// (`render rendering r : RenderingDef;`). sysml-rs currently parses only
// the inline form, so that is the form used here. The view-index must
// capture the rendering target ElementId so the renderer can consult the
// rendering's content.
//
// Two views bind to two different renderings; the diagram surface should
// be able to tell them apart by their rendering reference, not by name.

package Demo {
    part def Module {
        part child : Module[*];
    }
}

package Views {
    import Demo::*;
    import StandardViewDefinitions::*;

    rendering def DefaultRendering;
    rendering def CompactRendering;

    view def ExpandedView :> BrowserView {
        render rendering expandedStyle : DefaultRendering;
    }

    view def CompactedView :> BrowserView {
        render rendering compactStyle : CompactRendering;
    }

    view expanded : ExpandedView {
        expose Module;
    }

    view compacted : CompactedView {
        expose Module;
    }
}
