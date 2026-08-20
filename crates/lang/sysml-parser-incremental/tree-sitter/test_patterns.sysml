/* Mini test: one pattern per failing file. Test with:
   npx tree-sitter parse test_patterns.sysml 2>&1 | grep ERROR
*/

// === Group A: Binding [mult] source = [mult] target (4 files) ===
package BindingTests {
    // ControlPerformances pattern
    binding [1] whileDecision.ifTest = [1] whileTest;
    // TransitionPerformances pattern
    private binding [0..1] transitionLink.earlierOccurrence = [1] transitionLinkSource;
    // ShapeItems pattern (named + mult + ends)
    binding [1] bind [0..*] base.edges = [0..*] be;
    // Binding with "of"
    binding of [1] a.b = [1] c;
}

// === Group B: Disjoint X from Y (Occurrences.kerml) ===
package DisjointTests {
    abstract class Occ {
        disjoint earlierOccurrence.successors from laterOccurrence.predecessors;
        subset laterOccurrence.successors subsets earlierOccurrence.successors;
    }
}

// === Group C: end [mult] feature (Transfers.kerml) ===
package EndTests {
    connector c : BinaryLinkObject {
        end [1] feature transferSource references source;
        end [payloadNum] feature transferPayload references payload subsets transferSource.sourceOutput;
    }
}

// === Group D: default {expr} (Actions.sysml) ===
package DefaultTests {
    action def WhileLoop {
        in whileTest default {true} {
            doc /* A Boolean expression */
        }
    }
}

// === Group E: Expression context (5 files) ===
package ExpressionTests {
    private import ScalarValues::Natural;

    // Triggers: == comparison in body
    function TriggerAt {
        signalClock.currentTime == signalTime
    }
    // Observation: not operator in body
    struct IfThenElse {
        in bool redefines ifTest {
            not changeSignal.signalCondition()
        }
    }
    // Requirements: implies with body
    requirement def Req {
        return result = allTrue(assumptions()) implies allTrue(constraints()) {
            doc /* doc */
        }
    }
    // Interfaces: complex chained expression
    calc RemoveAt {
        private attribute position : Natural[1] = (1..size(seq))->selectOne{in i; seq#(i) == value};
        seq->excludingAt(position)
    }
    // StatePerformances: arrow + succession with mult
    state def StateP {
        step nonDoMiddle[*] subsets middle = middle->excluding(do);
        private succession [1] entry then [*] middle;
    }
}

// === Group F: out attribute (StateSpaceRepresentation.sysml) ===
package DirectionTests {
    private import ScalarValues::Integer;

    part def StateSpaceRepr {
        out attribute output : Integer = getOutput(input, stateSpace);
    }
}

// === Group G: doc in expression body (TradeStudies.sysml) ===
package DocInExprTests {
    attribute def Objective {
        attribute :>> best = alternatives->minimize {
            doc /* For a MinimizeObjective */
            in x; eval(x)
        };
    }
}

// === Group H: Function body with in params (CollectionFunctions.kerml) ===
package FunctionBodyTests {
    private import ScalarValues::Natural;
    private import ScalarValues::Positive;

    private function index { in arr : Array[1]; in i : Natural; in indexes : Positive[1..*];
        if i <= 1? indexes#(1) else arr.dimensions#(i) * (index(arr, i-1, indexes) - 1) + indexes#(i)
    }
}

// === Group I: Bracket after index (ISQSpaceTime.sysml) ===
package BracketTests {
    attribute x : LengthValue = num#(1) [mRef.mRefs#(1)];
}

// === Group J: Action control-flow nodes ===
action def ActionFlowTest {
    // Decision node
    decide decideNode;
    decide;

    // Fork node
    fork forkPoint;
    fork;

    // Join node
    join joinPoint;
    join;

    // Merge node
    merge mergePoint;
    merge;

    // Perform action
    perform action myAction;
    perform existingAction;
}
