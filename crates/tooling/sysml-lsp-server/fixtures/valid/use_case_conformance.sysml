// Conformance fixture: use case, case, actor, and include use case grammar.
// Tests that use_case_def, use_case_usage, case_def, case_usage,
// actor_usage, and include_use_case_usage all dispatch correctly.

package UseCaseConformance {

    part def Person;
    part def DataSet;

    // Use case definition with case body members
    use case def DriveToWork {
        subject driver : Person;
        actor passenger : Person;
        objective obj;
    }

    // Use case usage
    use case commute : DriveToWork;

    // Ref use case usage (with usage_prefix)
    ref use case self : DriveToWork;

    // Case definition with case body
    case def AnalyzeData {
        subject data : DataSet;
        actor analyst : Person;
        objective goal;
    }

    // Case usage
    case analysis : AnalyzeData;

    // Include use case (reference form)
    use case def ComplexWorkflow {
        include ref commute;
    }

    // Include use case (inline form)
    use case def FullWorkflow {
        include use case subStep : DriveToWork;
    }

    // Actor in requirement (cross-body validation)
    requirement def SafetyReq {
        actor driver : Person;
        require constraint {
            true
        }
    }
}
