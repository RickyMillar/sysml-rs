/* Fixture: systems_library_conformance.sysml
Intent: valid
Expected: systems library names (Requirements, Actions, States, Parts, Connections,
Constraints, Items, Ports, Allocations, Calculations, Cases, Views) resolve
through the salsa pipeline when the corresponding library graph is loaded.

This tests Sprint 2 Item 7 — verifying end-to-end name resolution for
standard systems library packages. */

package SystemsLibraryConformance {
    import Requirements::*;
    import Actions::*;
    import States::*;
    import Parts::*;
    import Connections::*;
    import Constraints::*;
    import Items::*;
    import Ports::*;
    import Allocations::*;
    import Calculations::*;
    import Cases::*;
    import Views::*;

    // Uses from Requirements library
    requirement def SafetyReq;

    // Uses from Actions library
    action def DriveAction;

    // Uses from States library
    state def OperatingState;

    // Uses from Parts library
    part def Vehicle;

    // Uses from Constraints library
    constraint def MassLimit;
}
