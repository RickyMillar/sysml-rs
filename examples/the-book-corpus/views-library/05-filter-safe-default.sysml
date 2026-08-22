// 05 — Filter safe-default fallthrough.
//
// Asserts: a filter expression the evaluator cannot reach (an unsupported
// operator, a dangling reference, an unimplemented type) falls through to
// `true`. The element is INCLUDED rather than excluded. The book's ch13
// "Filtering a View with `filter`" sidebar documents this contract:
// over-inclusion is safer than silent under-inclusion because the reader
// can see the surplus and investigate; they cannot see what is missing.
//
// The filter below uses an attribute reference that is not present on
// every element in the exposed scope. Elements where the reference does
// not resolve must still appear in the rendered view.

package Demo {
    attribute def Tag;

    part def WithTag {
        attribute t : Tag;
    }

    part def WithoutTag;

    part def Mixed {
        part wt : WithTag;
        part nt : WithoutTag;
    }
}

package Views {
    import Demo::*;
    import StandardViewDefinitions::*;

    view def TaggedView :> InterconnectionView {
        // `nt` (a `WithoutTag` instance) has no `t` attribute. Naive
        // evaluation would error or drop `nt`. Spec-faithful safe-default
        // includes it.
        filter t == "tagged";
    }

    view tagged : TaggedView {
        expose Mixed;
    }
}
