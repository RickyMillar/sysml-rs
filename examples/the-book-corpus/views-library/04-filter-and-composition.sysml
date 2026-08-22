// 04 — Filter clauses compose as logical AND.
//
// Asserts: two `filter` clauses on the same view both apply; an element
// must pass every filter to be included. Spec semantics for stacked
// `ElementFilterMembership`s on the same Namespace. The filters live in
// the view DEFINITION (the reusable recipe); the `expose` lives in the
// view USAGE (the concrete selection).

package Demo {
    part def AnalogPart;
    part def DigitalPart;

    part def Mixed {
        part a : AnalogPart;
        part d : DigitalPart;
        attribute label : String;
    }
}

package Views {
    import Demo::*;
    // No wildcard import of StandardViewDefinitions here: that package
    // re-exports SysML::*, which would make the bare `kind`/`name` filter
    // bindings ambiguous. Qualify the supertype instead.

    // Filter 1: only PartUsage elements (excludes attributes, definitions).
    // Filter 2: only elements whose name does not contain "Digital".
    // AND-composed: an element passes only if it is a PartUsage AND
    // does not contain "Digital" in its name.
    view def AnalogOnlyView :> StandardViewDefinitions::InterconnectionView {
        filter kind == "PartUsage";
        filter not (name == "DigitalPart");
    }

    view analogOnly : AnalogOnlyView {
        expose Mixed;
    }
}
