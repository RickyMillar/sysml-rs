// 01 — Minimal view definition and usage.
//
// Asserts: the smallest complete view renders end-to-end. A bare
// `view def` (no supertype — treated as the most general view) plus a
// `view` usage that exposes one element. Note which half carries the
// `expose`: the grammar admits `expose` only in a view USAGE body; a
// `view def` body takes only `filter` and `render`
// (SysML.xtext ViewDefinitionBodyItem has no expose alternative).

package Demo {
    part def Engine;
    part def Wheel;

    part def Vehicle {
        part engine : Engine;
        part wheels : Wheel[4];
    }
}

package Views {
    import Demo::*;

    view def MinimalView;

    view minimal : MinimalView {
        expose Vehicle;
    }
}
