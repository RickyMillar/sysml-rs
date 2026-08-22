// 02 — View usage instantiates a view definition.
//
// Asserts: `view <name> : <ViewDef>` instantiates the view definition
// and the supertype-driven shape (InterconnectionView, from the
// standard library package StandardViewDefinitions) flows through to
// the rendered diagram.

package Demo {
    part def Sensor;
    part def Controller;
    part def Actuator;

    part def ControlLoop {
        part s : Sensor;
        part c : Controller;
        part a : Actuator;
    }
}

package Views {
    import Demo::*;
    import StandardViewDefinitions::*;

    view def LoopView :> InterconnectionView {
        // Recipe: interconnection-shaped, no filter, no rendering binding.
    }

    // Two usages instantiate the same definition with different content.
    view loopOverview : LoopView {
        expose ControlLoop;
    }

    view sensorDetail : LoopView {
        expose ControlLoop::s;
    }
}
