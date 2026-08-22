// 03 — Namespace expose vs member expose.
//
// Asserts: `expose Foo::*` (namespace expose) imports every member of
// the namespace; `expose Foo` (member expose) imports only the named
// element. Both are spec-faithful; sysml-rs distinguishes them via the
// `isNamespace` flag captured by the view index.

package Demo {
    package Subsystem {
        part def Compressor;
        part def Pump;
        part def Reservoir;
    }

    part def StandalonePart;
}

package Views {
    import Demo::*;
    import StandardViewDefinitions::*;

    // Namespace expose — pulls Compressor, Pump, Reservoir.
    view def AllOfSubsystem :> BrowserView {
        // No filter; the usage's expose alone scopes the content.
    }

    view allOfSubsystem : AllOfSubsystem {
        expose Subsystem::*;
    }

    // Member expose — pulls a single element by qname.
    view def JustStandalone :> BrowserView;

    view justStandalone : JustStandalone {
        expose StandalonePart;
    }
}
