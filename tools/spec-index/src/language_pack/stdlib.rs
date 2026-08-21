//! Standard-library semantic cards (Tier 4 sources /
//! `library-defined`). A curated table of the most load-bearing constructs
//! whose *meaning* comes from the normative standard model library, not from
//! the grammar: the verification-verdict / case / requirement-check /
//! constraint-check machinery. Per root CLAUDE.md source precedence 2, the
//! library models ARE the semantics of these constructs, so each card cites the
//! defining library file (Tier 4, allowlisted in [`super::manifest`]) plus the
//! spec clause that documents it, and carries no grammar IR (`library-defined`,
//! not a concrete-syntax production).
//!
//! Support stays honest-`unknown`: these cards have no positive/negative parse
//! examples (the concept is a library definition, not a syntax rule), so the
//! example-evidence pipeline that drives [`super::support`] emits nothing for
//! them — the meaning is carried by the cited library file + clause, never a
//! fabricated axis value.

use super::manifest;

/// One curated standard-library card.
pub struct StdlibCard {
    /// Concept ID (`<authority>.library.<slug>`); authority = the library layer
    /// (`sysml` = Systems Library, `kerml` = Kernel Semantic Library).
    pub id: &'static str,
    pub title: &'static str,
    /// `SysML` | `KerML` (schema `language`).
    pub language: &'static str,
    /// The library element name (used as an alias + retrieval keyword).
    pub element: &'static str,
    /// Original paraphrase (never reproduced normative prose).
    pub summary: &'static str,
    /// The allowlisted (Tier 4) library file that defines this construct.
    pub library_path: &'static str,
    /// `SysML` | `KerML` — which derived spec-text index to resolve `clause` in.
    pub document: &'static str,
    /// Spec clause documenting the construct; try-resolved against the heading
    /// index and included only if it resolves (never a faked citation).
    pub clause: &'static str,
    /// Obligation / semantic-rule IDs this library definition backs (e.g. the
    /// `verdict-kind-enumeration` obligation that checks VerdictKind's closed
    /// set). Empty for constructs with no dedicated well-formedness gate.
    pub validation_rules: &'static [&'static str],
    /// Cross-links (must resolve to existing cards). Made bidirectional by the
    /// generator.
    pub related_cards: &'static [&'static str],
    /// Extra retrieval keywords beyond the element name + title words.
    pub keywords_extra: &'static [&'static str],
}

/// The curated set. Kept deliberately small and load-bearing (the
/// verdict / case / check machinery the milestone named), each grounded in a
/// named Tier 4 library file. Extend only with equally load-bearing constructs.
pub fn stdlib_cards() -> &'static [StdlibCard] {
    &[
        StdlibCard {
            id: "sysml.library.verdict-kind",
            title: "Verdict Kind",
            language: "SysML",
            element: "VerdictKind",
            summary: "The library enumeration of the possible results of a verification case: \
                      `pass`, `fail`, `inconclusive`, and `error`. A VerificationCase returns a \
                      VerdictKind as its result.",
            library_path: manifest::STDLIB_VERIFICATION_CASES,
            document: "SysML",
            clause: "7.24",
            validation_rules: &["verdict-kind-enumeration"],
            related_cards: &[
                "sysml.library.verification-case",
                "sysml.library.pass-if",
            ],
            keywords_extra: &["verdict", "pass", "fail", "inconclusive", "error", "enumeration"],
        },
        StdlibCard {
            id: "sysml.library.verification-case",
            title: "Verification Case (Library Definition)",
            language: "SysML",
            element: "VerificationCase",
            summary: "The abstract library base of all verification cases (a specialization of \
                      Case). It returns a VerdictKind verdict and records the RequirementChecks of \
                      the requirements being verified in its objective.",
            library_path: manifest::STDLIB_VERIFICATION_CASES,
            document: "SysML",
            clause: "7.24",
            validation_rules: &[],
            related_cards: &[
                "sysml.library.verdict-kind",
                "sysml.library.case",
                "sysml.cases.verification-case",
            ],
            keywords_extra: &["verification", "case", "requirementverifications", "objective"],
        },
        StdlibCard {
            id: "sysml.library.pass-if",
            title: "Pass If",
            language: "SysML",
            element: "PassIf",
            summary: "The library calculation mapping a Boolean to a VerdictKind: it returns \
                      `VerdictKind::pass` when its `isPassing` argument is true, otherwise \
                      `VerdictKind::fail`.",
            library_path: manifest::STDLIB_VERIFICATION_CASES,
            document: "SysML",
            clause: "7.24",
            validation_rules: &[],
            related_cards: &["sysml.library.verdict-kind"],
            keywords_extra: &["passif", "calculation", "boolean", "verdict"],
        },
        StdlibCard {
            id: "sysml.library.verification-method-kind",
            title: "Verification Method Kind",
            language: "SysML",
            element: "VerificationMethodKind",
            summary: "The library enumeration of the standard methods by which verification can be \
                      carried out: `inspect`, `analyze`, `demo`, and `test`. Used via the \
                      VerificationMethod metadata annotating a verification case or action.",
            library_path: manifest::STDLIB_VERIFICATION_CASES,
            document: "SysML",
            clause: "7.24",
            validation_rules: &[],
            related_cards: &["sysml.library.verification-case"],
            keywords_extra: &[
                "verification", "method", "inspect", "analyze", "demo", "test", "metadata",
            ],
        },
        StdlibCard {
            id: "sysml.library.case",
            title: "Case (Library Definition)",
            language: "SysML",
            element: "Case",
            summary: "The abstract library base of all cases (a specialization of Calculation). A \
                      Case has a subject under investigation, actor parts, an objective expressed \
                      as a RequirementCheck, and a result that should satisfy that objective.",
            library_path: manifest::STDLIB_CASES,
            document: "SysML",
            clause: "7.22",
            validation_rules: &[],
            related_cards: &[
                "sysml.library.requirement-check",
                "sysml.library.analysis-case",
                "sysml.library.verification-case",
            ],
            keywords_extra: &["case", "subject", "objective", "actors", "result", "calculation"],
        },
        StdlibCard {
            id: "sysml.library.analysis-case",
            title: "Analysis Case (Library Definition)",
            language: "SysML",
            element: "AnalysisCase",
            summary: "The abstract library base of all analysis cases (a specialization of Case). \
                      It carries out an evaluation over its subject, producing a result rather than \
                      a pass/fail verdict.",
            library_path: manifest::STDLIB_ANALYSIS_CASES,
            document: "SysML",
            clause: "7.23",
            validation_rules: &[],
            related_cards: &["sysml.library.case", "sysml.cases.analysis-case"],
            keywords_extra: &["analysis", "case", "evaluation", "subject"],
        },
        StdlibCard {
            id: "sysml.library.requirement-check",
            title: "Requirement Check",
            language: "SysML",
            element: "RequirementCheck",
            summary: "The abstract library base of all requirement definitions (a specialization \
                      of RequirementConstraintCheck). It checks whether its subject satisfies the \
                      required constraints, given that all assumptions hold: its result is \
                      `allTrue(assumptions) implies allTrue(constraints)`.",
            library_path: manifest::STDLIB_REQUIREMENTS,
            document: "SysML",
            clause: "7.21",
            // The `implies` semantics above ARE the normative home of the
            // vacuous-satisfaction obligation, so this card is that obligation's
            // locator (the obligation folds into it).
            validation_rules: &["assumption-false-required-vacuously-satisfied"],
            related_cards: &[
                "sysml.library.constraint-check",
                "sysml.requirements.requirement-definition",
            ],
            keywords_extra: &[
                "requirement", "check", "assumptions", "constraints", "subject", "satisfy",
            ],
        },
        StdlibCard {
            id: "sysml.library.constraint-check",
            title: "Constraint Check",
            language: "SysML",
            element: "ConstraintCheck",
            summary: "The abstract library base of all constraint definitions (a specialization of \
                      the Kernel BooleanEvaluation predicate). A ConstraintCheck evaluates to a \
                      Boolean; asserted and negated constraint checks partition it into the \
                      true/false evaluation subsets.",
            library_path: manifest::STDLIB_CONSTRAINTS,
            document: "SysML",
            clause: "7.20",
            validation_rules: &[],
            related_cards: &[
                "kerml.library.boolean-evaluation",
                "sysml.requirements.constraint-definition",
            ],
            keywords_extra: &["constraint", "check", "boolean", "evaluation", "asserted", "negated"],
        },
        // Obligation-home cards: the normative home of three obligations
        // whose citation cell pointed at a library file (no resolvable spec
        // clause), so they were blocked. Each stdlib card is that obligation's
        // locator — the obligation folds into it instead of
        // staying blocked.
        StdlibCard {
            id: "sysml.library.calculation",
            title: "Calculation (Library Definition)",
            language: "SysML",
            element: "Calculation",
            summary: "The abstract library base of all calculations (a specialization of Action \
                      and the Kernel Evaluation). A CalculationUsage evaluates an expression over \
                      its parameters and returns a result; when an argument is omitted, the \
                      declared parameter's default value (a KerML FeatureValue) is used.",
            library_path: manifest::STDLIB_CALCULATIONS,
            document: "SysML",
            clause: "7.19",
            // The default-parameter semantics above ARE this obligation's home.
            validation_rules: &["calc-default-param-applied-when-arg-absent"],
            related_cards: &["kerml.expression.feature-value"],
            keywords_extra: &["calculation", "parameter", "default", "featurevalue", "evaluation"],
        },
        StdlibCard {
            id: "kerml.library.state-performance",
            title: "State Performance",
            language: "KerML",
            element: "StatePerformance",
            summary: "The Kernel Semantic Library performance of being in a state (a specialization \
                      of DecisionPerformance) with entry/do/exit substeps. On entry, the \
                      transfer/event that triggered the entry is recorded on the state's \
                      performance via its `incomingTransitionTrigger` feature.",
            library_path: manifest::STDLIB_STATE_PERFORMANCES,
            document: "KerML",
            clause: "9.2.11",
            validation_rules: &["incoming-transition-trigger-recorded"],
            related_cards: &[],
            keywords_extra: &[
                "state", "performance", "entry", "exit", "incomingtransitiontrigger", "transition",
            ],
        },
        StdlibCard {
            id: "sysml.library.state-action",
            title: "State Action (Library Definition)",
            language: "SysML",
            element: "StateAction",
            summary: "The abstract library base of all state usages (a specialization of Action and \
                      the Kernel StatePerformance). Its mutually-exclusive substates are sequenced \
                      by `stateSequencing` successions: with N exclusive substates there are \
                      exactly N-1 such successions.",
            library_path: manifest::STDLIB_STATES,
            document: "SysML",
            clause: "7.18",
            validation_rules: &["state-sequencing-count-invariant"],
            related_cards: &["kerml.library.state-performance"],
            keywords_extra: &["state", "action", "statesequencing", "exclusive", "substate"],
        },
        StdlibCard {
            id: "kerml.library.boolean-evaluation",
            title: "Boolean Evaluation",
            language: "KerML",
            element: "BooleanEvaluation",
            summary: "The Kernel Semantic Library predicate that is the most general class of \
                      Boolean-valued evaluations (a specialization of Evaluation). It is the base \
                      that the Systems Library ConstraintCheck specializes.",
            library_path: manifest::STDLIB_PERFORMANCES,
            document: "KerML",
            clause: "9.2.6",
            validation_rules: &[],
            related_cards: &["sysml.library.constraint-check"],
            keywords_extra: &["boolean", "evaluation", "predicate", "performance", "kernel"],
        },
        // ===================================================================
        // Member-level cards — the highest-retrieval-value library members
        // an LLM reaches for when authoring expressions, flows, timing, and
        // quantities. Support stays honest-`unknown` (library definitions, no
        // parse examples). Each cites its Kernel/Domain library file + clause.
        // ===================================================================
        // --- Scalar functions (Kernel Function Library, KerML 9.4.4) ---
        StdlibCard {
            id: "kerml.library.scalar-max",
            title: "max (Scalar Function)",
            language: "KerML",
            element: "max",
            summary: "The Kernel scalar function returning whichever of its two ordered scalar \
                      operands is the greater. Invoked as `max(a, b)` in expressions.",
            library_path: manifest::STDLIB_SCALAR_FUNCTIONS,
            document: "KerML",
            clause: "9.4.4",
            validation_rules: &[],
            related_cards: &["kerml.library.scalar-min"],
            keywords_extra: &["max", "maximum", "greater", "scalar", "function", "comparison"],
        },
        StdlibCard {
            id: "kerml.library.scalar-min",
            title: "min (Scalar Function)",
            language: "KerML",
            element: "min",
            summary: "The Kernel scalar function returning whichever of its two ordered scalar \
                      operands is the lesser. Invoked as `min(a, b)` in expressions.",
            library_path: manifest::STDLIB_SCALAR_FUNCTIONS,
            document: "KerML",
            clause: "9.4.4",
            validation_rules: &[],
            related_cards: &["kerml.library.scalar-max"],
            keywords_extra: &["min", "minimum", "lesser", "scalar", "function", "comparison"],
        },
        // --- Sequence / collection functions (Kernel Function Library, KerML 9.4.14) ---
        StdlibCard {
            id: "kerml.library.size",
            title: "size (Sequence Function)",
            language: "KerML",
            element: "size",
            summary: "The Kernel sequence function counting how many elements a collection holds, \
                      returning a Natural. `col->size()` is the idiomatic length query.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.is-empty"],
            keywords_extra: &["size", "count", "length", "sequence", "collection", "cardinality"],
        },
        StdlibCard {
            id: "kerml.library.is-empty",
            title: "isEmpty (Sequence Function)",
            language: "KerML",
            element: "isEmpty",
            summary: "The Kernel sequence predicate that is true exactly when a collection contains \
                      no elements. Complementary to `notEmpty`.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.not-empty", "kerml.library.size"],
            keywords_extra: &["isempty", "empty", "sequence", "collection", "predicate"],
        },
        StdlibCard {
            id: "kerml.library.not-empty",
            title: "notEmpty (Sequence Function)",
            language: "KerML",
            element: "notEmpty",
            summary: "The Kernel sequence predicate that is true when a collection holds at least \
                      one element — the negation of `isEmpty`.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.is-empty"],
            keywords_extra: &["notempty", "nonempty", "sequence", "collection", "predicate"],
        },
        StdlibCard {
            id: "kerml.library.includes",
            title: "includes (Sequence Function)",
            language: "KerML",
            element: "includes",
            summary: "The Kernel sequence predicate that is true when every element of one \
                      collection is also present in another — a membership/containment test.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.size"],
            keywords_extra: &["includes", "contains", "membership", "sequence", "collection"],
        },
        StdlibCard {
            id: "kerml.library.head",
            title: "head (Sequence Function)",
            language: "KerML",
            element: "head",
            summary: "The Kernel sequence function returning the first element of an ordered \
                      collection (equivalent to indexing position 1), or nothing when it is empty.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.tail", "kerml.library.last"],
            keywords_extra: &["head", "first", "front", "ordered", "sequence"],
        },
        StdlibCard {
            id: "kerml.library.tail",
            title: "tail (Sequence Function)",
            language: "KerML",
            element: "tail",
            summary: "The Kernel sequence function returning every element of an ordered collection \
                      except the first — the complement of `head`, used for recursive traversal.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.head"],
            keywords_extra: &["tail", "rest", "remainder", "ordered", "sequence"],
        },
        StdlibCard {
            id: "kerml.library.last",
            title: "last (Sequence Function)",
            language: "KerML",
            element: "last",
            summary: "The Kernel sequence function returning the final element of an ordered \
                      collection, or nothing when it is empty.",
            library_path: manifest::STDLIB_SEQUENCE_FUNCTIONS,
            document: "KerML",
            clause: "9.4.14",
            validation_rules: &[],
            related_cards: &["kerml.library.head"],
            keywords_extra: &["last", "final", "back", "ordered", "sequence"],
        },
        // --- Transfers (Kernel Semantic Library, KerML 9.2.7) ---
        StdlibCard {
            id: "kerml.library.transfer",
            title: "Transfer",
            language: "KerML",
            element: "Transfer",
            summary: "The Kernel interaction that moves a payload from a source occurrence to a \
                      target occurrence. It is the abstract base every flow and message transfer \
                      specializes; a transfer may be instantaneous or take time.",
            library_path: manifest::STDLIB_TRANSFERS,
            document: "KerML",
            clause: "9.2.7",
            validation_rules: &[],
            related_cards: &[
                "kerml.library.flow-transfer",
                "kerml.library.message-transfer",
                "kerml.library.occurrence",
            ],
            keywords_extra: &["transfer", "payload", "source", "target", "interaction", "flow"],
        },
        StdlibCard {
            id: "kerml.library.flow-transfer",
            title: "FlowTransfer",
            language: "KerML",
            element: "FlowTransfer",
            summary: "The concrete Transfer that names the source output feature and the target \
                      input feature the payload flows between — what an ordinary structural flow \
                      connection lowers to. Carries move/push semantics.",
            library_path: manifest::STDLIB_TRANSFERS,
            document: "KerML",
            clause: "9.2.7",
            validation_rules: &[],
            related_cards: &["kerml.library.transfer", "kerml.library.message-transfer"],
            keywords_extra: &["flowtransfer", "flow", "ismove", "ispush", "connection", "transfer"],
        },
        StdlibCard {
            id: "kerml.library.message-transfer",
            title: "MessageTransfer",
            language: "KerML",
            element: "MessageTransfer",
            summary: "The Transfer variant that carries a payload with no named source-output or \
                      target-input feature — the transfer underlying send/accept action semantics. \
                      It is disjoint from FlowTransfer.",
            library_path: manifest::STDLIB_TRANSFERS,
            document: "KerML",
            clause: "9.2.7",
            validation_rules: &[],
            related_cards: &["kerml.library.transfer", "kerml.library.flow-transfer"],
            keywords_extra: &["messagetransfer", "message", "send", "accept", "transfer", "disjoint"],
        },
        // --- Occurrences (Kernel Semantic Library, KerML 9.2.4) ---
        StdlibCard {
            id: "kerml.library.occurrence",
            title: "Occurrence",
            language: "KerML",
            element: "Occurrence",
            summary: "The Kernel root classifier for anything with identity that exists or happens \
                      across time and space. Actions, states, and individual parts ultimately \
                      specialize it; it anchors the timing (start/end snapshots) and transfer \
                      features every temporal element uses.",
            library_path: manifest::STDLIB_OCCURRENCES,
            document: "KerML",
            clause: "9.2.4",
            validation_rules: &[],
            related_cards: &[
                "kerml.library.happens-before",
                "kerml.library.happens-during",
                "kerml.library.transfer",
            ],
            keywords_extra: &["occurrence", "individual", "time", "space", "startshot", "endshot"],
        },
        StdlibCard {
            id: "kerml.library.happens-before",
            title: "HappensBefore",
            language: "KerML",
            element: "HappensBefore",
            summary: "The Kernel association asserting that one occurrence completely finishes \
                      before another begins, with no overlap in time — the semantic backbone of \
                      succession (`then`) ordering.",
            library_path: manifest::STDLIB_OCCURRENCES,
            document: "KerML",
            clause: "9.2.4",
            validation_rules: &[],
            related_cards: &["kerml.library.occurrence", "kerml.library.happens-during"],
            keywords_extra: &["happensbefore", "succession", "then", "ordering", "before", "timing"],
        },
        StdlibCard {
            id: "kerml.library.happens-during",
            title: "HappensDuring",
            language: "KerML",
            element: "HappensDuring",
            summary: "The Kernel association asserting that one occurrence's whole time interval \
                      falls inside another's — used for containment timing, e.g. a substate being \
                      active during its enclosing state.",
            library_path: manifest::STDLIB_OCCURRENCES,
            document: "KerML",
            clause: "9.2.4",
            validation_rules: &[],
            related_cards: &["kerml.library.occurrence", "kerml.library.happens-before"],
            keywords_extra: &["happensduring", "during", "containment", "interval", "timing"],
        },
        // --- SI units (Domain Libraries, SysML 9.8.6) ---
        StdlibCard {
            id: "sysml.library.metre",
            title: "metre (SI Unit)",
            language: "SysML",
            element: "metre",
            summary: "The SI base unit of length (short name `m`), typing LengthValue quantities. \
                      Imported from the SI library and composed into derived units such as newton.",
            library_path: manifest::STDLIB_SI,
            document: "SysML",
            clause: "9.8.6",
            validation_rules: &[],
            related_cards: &["sysml.library.length-value"],
            keywords_extra: &["metre", "meter", "m", "length", "si", "unit"],
        },
        StdlibCard {
            id: "sysml.library.kilogram",
            title: "kilogram (SI Unit)",
            language: "SysML",
            element: "kilogram",
            summary: "The SI base unit of mass (short name `kg`), typing MassValue quantities. \
                      Defined by applying the `kilo` prefix to the gram.",
            library_path: manifest::STDLIB_SI,
            document: "SysML",
            clause: "9.8.6",
            validation_rules: &[],
            related_cards: &["sysml.library.mass-value"],
            keywords_extra: &["kilogram", "kg", "mass", "si", "unit", "kilo"],
        },
        StdlibCard {
            id: "sysml.library.second",
            title: "second (SI Unit)",
            language: "SysML",
            element: "second",
            summary: "The SI base unit of time/duration (short name `s`), typing DurationValue \
                      quantities. The reference unit for rates and time-based dynamics.",
            library_path: manifest::STDLIB_SI,
            document: "SysML",
            clause: "9.8.6",
            validation_rules: &[],
            related_cards: &["sysml.library.duration-value"],
            keywords_extra: &["second", "s", "time", "duration", "si", "unit"],
        },
        // --- ISQ base quantities (Domain Libraries, SysML 9.8.4) ---
        StdlibCard {
            id: "sysml.library.length-value",
            title: "LengthValue (ISQ Base Quantity)",
            language: "SysML",
            element: "LengthValue",
            summary: "The ISQ base quantity kind for length (dimension L). A LengthValue is a \
                      scalar quantity measured in a LengthUnit such as metre.",
            library_path: manifest::STDLIB_ISQ_BASE,
            document: "SysML",
            clause: "9.8.4",
            validation_rules: &[],
            related_cards: &["sysml.library.metre"],
            keywords_extra: &["lengthvalue", "length", "isq", "quantity", "dimension", "l"],
        },
        StdlibCard {
            id: "sysml.library.mass-value",
            title: "MassValue (ISQ Base Quantity)",
            language: "SysML",
            element: "MassValue",
            summary: "The ISQ base quantity kind for mass (dimension M). A MassValue is a scalar \
                      quantity measured in a MassUnit such as kilogram.",
            library_path: manifest::STDLIB_ISQ_BASE,
            document: "SysML",
            clause: "9.8.4",
            validation_rules: &[],
            related_cards: &["sysml.library.kilogram"],
            keywords_extra: &["massvalue", "mass", "isq", "quantity", "dimension", "m"],
        },
        StdlibCard {
            id: "sysml.library.duration-value",
            title: "DurationValue (ISQ Base Quantity)",
            language: "SysML",
            element: "DurationValue",
            summary: "The ISQ base quantity kind for duration/time (dimension T). A DurationValue \
                      is a scalar quantity measured in a DurationUnit such as second.",
            library_path: manifest::STDLIB_ISQ_BASE,
            document: "SysML",
            clause: "9.8.4",
            validation_rules: &[],
            related_cards: &["sysml.library.second"],
            keywords_extra: &["durationvalue", "duration", "time", "isq", "quantity", "dimension", "t"],
        },
    ]
}
