//! Held-out executable evaluation datasets.
//!
//! Four datasets, authored fresh (not copied from the pack examples) and
//! serialized deterministically into `evals/`:
//!
//! - **explanation** — concept questions with the card ids a correct
//!   answer must cite and the key facts it must state.
//! - **generation** — modelling tasks whose reference solution must
//!   parse green through the real tree-sitter parser.
//! - **repair** — a broken snippet, the phase/category it must fail
//!   at, and a fixed reference that parses/validates cleanly. Variants are
//!   authored fresh; they do not reuse any pack example source verbatim.
//! - **support-discrimination** — questions that separate "valid
//!   SysML" from "supported by sysml-rs", answerable from the support axes.
//!
//! The datasets are authored data, not derived from the cards, so they hold no
//! secret knowledge of the corpus — the `sysml-spec-tests::language_pack_evals`
//! gate proves every reference answer passes its own check (references parse,
//! cited cards exist, expected diagnostics fire, support values match the live
//! pack). Emitted as JSONL, one record per line, sorted by id.

use serde::Serialize;

use super::LpError;

/// A concept-explanation question.
#[derive(Debug, Clone, Serialize)]
struct ExplanationEval {
    id: String,
    question: String,
    /// Card ids a correct answer must cite (all must exist in the pack).
    expected_card_ids: Vec<String>,
    /// Normative locators (`"<Document> <clause>"`) a correct answer must ground
    /// itself in. Each MUST appear in a cited card's `normative_clauses`, so the
    /// answer key is anchored to the spec, not just to non-empty prose.
    normative_locators: Vec<String>,
    /// Substrings a correct answer should contain (graded facts).
    key_facts: Vec<String>,
    /// True when the question turns on the normative-vs-implementation line.
    distinguishes_normative_vs_implementation: bool,
}

/// A generation task with a reference solution checked through its
/// declared phases.
#[derive(Debug, Clone, Serialize)]
struct GenerationEval {
    id: String,
    task: String,
    expected_card_ids: Vec<String>,
    /// Reference SysML that MUST parse with zero syntax errors.
    reference_solution: String,
    /// When true, the reference is self-contained and MUST also resolve with
    /// zero unresolved references (the runner checks parse + resolve, not just
    /// parse). Left false when the snippet intentionally leans on library types
    /// not loaded in the isolated eval pipeline.
    check_resolve: bool,
}

/// A syntax/semantic repair item.
#[derive(Debug, Clone, Serialize)]
struct RepairEval {
    id: String,
    broken_source: String,
    /// Phase the broken source must fail at: parse | resolve | validate.
    expected_phase: String,
    /// Mutation class.
    expected_diagnostic_category: String,
    /// Specific validator id for validate-phase items (S0xx), else null.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic_code: Option<String>,
    /// Reference fix that MUST parse/validate cleanly.
    fixed_source: String,
    expected_card_ids: Vec<String>,
}

/// A normative-vs-implementation discrimination question.
#[derive(Debug, Clone, Serialize)]
struct SupportDiscriminationEval {
    id: String,
    question: String,
    concept_card_id: String,
    /// One of the 8 support axes.
    axis: String,
    /// Expected support value — asserted equal to the card's live axis so the
    /// dataset cannot silently drift from the pack.
    expected_support: String,
    answer_key: String,
}

fn s(x: &str) -> String {
    x.to_owned()
}
fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| (*x).to_owned()).collect()
}

fn explanation() -> Vec<ExplanationEval> {
    vec![
        // --- SysML structure ------------------------------------------
        ExplanationEval {
            id: s("exp.part-usage-vs-def"),
            question: s("What is the difference between a part definition and a part usage in SysML v2?"),
            expected_card_ids: v(&["sysml.structure.part-definition", "sysml.structure.part-usage"]),
            normative_locators: v(&["SysML 8.3.11.2", "SysML 8.3.11.3"]),
            key_facts: v(&["part def", "part", "usage", "typed by"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.connection-usage"),
            question: s("How do you connect two parts together in SysML v2?"),
            expected_card_ids: v(&["sysml.structure.connection-usage"]),
            normative_locators: v(&["SysML 8.3.13.4"]),
            key_facts: v(&["connect", "connection", "to"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.port-flow"),
            question: s("How do ports let two parts exchange something in SysML v2?"),
            expected_card_ids: v(&["sysml.structure.port-definition", "sysml.structure.flow-connection"]),
            normative_locators: v(&["SysML 8.3.12.5", "SysML 8.3.16.3"]),
            key_facts: v(&["port", "flow", "connect"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.enum-def"),
            question: s("How do you declare an enumeration with named values in SysML v2?"),
            expected_card_ids: v(&["sysml.structure.enumeration-definition"]),
            normative_locators: v(&["SysML 8.3.8.2"]),
            key_facts: v(&["enum def", "enum", "value"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.attribute-usage"),
            question: s("What is an attribute usage and how does it differ from an attribute definition?"),
            expected_card_ids: v(&["sysml.structure.attribute-definition", "sysml.structure.attribute-usage"]),
            normative_locators: v(&["SysML 8.3.7.2", "SysML 8.3.7.3"]),
            key_facts: v(&["attribute", "attribute def", "value", "typed"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.item-vs-part"),
            question: s("When would you use an item definition instead of a part definition?"),
            expected_card_ids: v(&["sysml.structure.item-definition", "sysml.structure.part-definition"]),
            normative_locators: v(&["SysML 8.3.10.2", "SysML 8.3.11.2"]),
            key_facts: v(&["item", "part", "flows", "occurrence"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.interface"),
            question: s("What does an interface definition connect in SysML v2?"),
            expected_card_ids: v(&["sysml.structure.interface-definition"]),
            normative_locators: v(&["SysML 8.3.14.2"]),
            key_facts: v(&["interface", "port", "connect"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- KerML core / relationships -------------------------------
        ExplanationEval {
            id: s("exp.specialization"),
            question: s("What does specialization mean between two types in KerML?"),
            expected_card_ids: v(&["kerml.structure.specialization"]),
            normative_locators: v(&["KerML 8.3.3.1.8"]),
            key_facts: v(&["specialization", "subtype", "inherit"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.subsetting-vs-redefinition"),
            question: s("What is the difference between subsetting and redefinition of a feature?"),
            expected_card_ids: v(&["kerml.structure.subsetting", "kerml.structure.redefinition"]),
            normative_locators: v(&["KerML 8.3.3.3.10", "KerML 8.3.3.3.8"]),
            key_facts: v(&["subsets", "redefines", "feature", "inherit"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.feature-typing"),
            question: s("How is a feature given a type in KerML?"),
            expected_card_ids: v(&["kerml.structure.feature-typing"]),
            normative_locators: v(&["KerML 8.3.3.3.7"]),
            key_facts: v(&["typing", "typed by", "feature"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.multiplicity"),
            question: s("How does multiplicity express how many values a feature can have?"),
            expected_card_ids: v(&["kerml.structure.multiplicity"]),
            normative_locators: v(&["KerML 8.3.3.1.9"]),
            key_facts: v(&["multiplicity", "bound", "cardinality"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.import"),
            question: s("How does one package make another package's members visible in SysML v2?"),
            expected_card_ids: v(&["kerml.structure.import"]),
            normative_locators: v(&["KerML 8.3.2.4.2"]),
            key_facts: v(&["import", "namespace", "visible"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.association"),
            question: s("What is an association in KerML and what are its ends?"),
            expected_card_ids: v(&["kerml.structure.association"]),
            normative_locators: v(&["KerML 7.4.5"]),
            key_facts: v(&["association", "end", "link"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.package-namespace"),
            question: s("How does a package organize members in a namespace?"),
            expected_card_ids: v(&["kerml.structure.package", "kerml.structure.namespace"]),
            normative_locators: v(&["KerML 7.4.14", "KerML 8.3.2.4.5"]),
            key_facts: v(&["package", "namespace", "member"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Expressions ----------------------------------------------
        ExplanationEval {
            id: s("exp.literal-integer"),
            question: s("How is an integer literal written in a SysML expression?"),
            expected_card_ids: v(&["kerml.expression.literal-integer"]),
            normative_locators: v(&["KerML 8.3.4.8.12"]),
            key_facts: v(&["integer", "literal", "number"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.invocation"),
            question: s("How does an invocation expression pass arguments to a calculation?"),
            expected_card_ids: v(&["kerml.expression.invocation"]),
            normative_locators: v(&["KerML 8.3.4.8.8"]),
            key_facts: v(&["invocation", "argument", "calc"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.feature-reference"),
            question: s("What does a feature reference expression evaluate to?"),
            expected_card_ids: v(&["kerml.expression.feature-reference"]),
            normative_locators: v(&["KerML 8.3.4.8.5"]),
            key_facts: v(&["feature", "reference", "value"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Action nodes ---------------------------------------------
        ExplanationEval {
            id: s("exp.action-def"),
            question: s("What is an action definition in SysML v2?"),
            expected_card_ids: v(&["sysml.behavior.action-definition", "sysml.behavior.action-usage"]),
            normative_locators: v(&["SysML 8.3.17.3", "SysML 8.3.17.4"]),
            key_facts: v(&["action", "action def", "behavior"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.accept-node"),
            question: s("What does an accept node do in an action in SysML v2?"),
            expected_card_ids: v(&["sysml.behavior.accept-node"]),
            normative_locators: v(&["SysML 7.17.8"]),
            key_facts: v(&["accept", "receive", "payload"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.decision-node"),
            question: s("How does a decision node choose an outgoing flow?"),
            expected_card_ids: v(&["sysml.behavior.decision-node"]),
            normative_locators: v(&["SysML 7.17.3"]),
            key_facts: v(&["decision", "guard", "outgoing"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.fork-vs-join"),
            question: s("What is the difference between a fork node and a join node?"),
            expected_card_ids: v(&["sysml.behavior.fork-node", "sysml.behavior.join-node"]),
            normative_locators: v(&["SysML 7.17.3"]),
            key_facts: v(&["fork", "join", "concurrent", "synchronize"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.calculation"),
            question: s("What does a calculation definition compute and return?"),
            expected_card_ids: v(&["sysml.behavior.calculation-definition"]),
            normative_locators: v(&["SysML 8.3.19.2"]),
            key_facts: v(&["calc", "calculation", "return"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- States ---------------------------------------------------
        ExplanationEval {
            id: s("exp.transition-usage"),
            question: s("How is a state transition written in SysML v2 textual notation?"),
            expected_card_ids: v(&["sysml.behavior.transition-usage"]),
            normative_locators: v(&["SysML 8.3.18.9"]),
            key_facts: v(&["transition", "then", "trigger"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.state-def"),
            question: s("What is a state definition and how do its states relate?"),
            expected_card_ids: v(&["sysml.behavior.state-definition", "sysml.behavior.state-usage"]),
            normative_locators: v(&["SysML 8.3.18.5", "SysML 8.3.18.6"]),
            key_facts: v(&["state", "state def", "machine"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.exhibit-state"),
            question: s("What does exhibiting a state mean for a part in SysML v2?"),
            expected_card_ids: v(&["sysml.behavior.exhibit-state"]),
            normative_locators: v(&["SysML 8.3.18.2"]),
            key_facts: v(&["exhibit", "state", "behavior"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Requirements / cases -------------------------------------
        ExplanationEval {
            id: s("exp.requirement-subject"),
            question: s("What role does the subject play in a requirement definition?"),
            expected_card_ids: v(&["sysml.requirements.requirement-definition", "sysml.requirements.subject"]),
            normative_locators: v(&["SysML 8.3.21.8", "SysML 8.3.21.11"]),
            key_facts: v(&["subject", "requirement", "first parameter"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.constraint"),
            question: s("How does a constraint definition express a boolean condition?"),
            expected_card_ids: v(&["sysml.requirements.constraint-definition", "sysml.requirements.constraint-usage"]),
            normative_locators: v(&["SysML 8.3.20.3", "SysML 8.3.20.4"]),
            key_facts: v(&["constraint", "boolean", "condition"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.actor-stakeholder"),
            question: s("What is the difference between an actor and a stakeholder in requirements?"),
            expected_card_ids: v(&["sysml.requirements.actor", "sysml.requirements.stakeholder"]),
            normative_locators: v(&["SysML 8.3.21.2", "SysML 8.3.21.12"]),
            key_facts: v(&["actor", "stakeholder", "role", "concern"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.satisfaction"),
            question: s("How does a part satisfy a requirement in SysML v2?"),
            expected_card_ids: v(&["sysml.requirements.satisfaction"]),
            normative_locators: v(&["SysML 8.3.21.10"]),
            key_facts: v(&["satisfy", "requirement", "by"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.verification-case"),
            question: s("What is a verification case and how does it relate to a requirement?"),
            expected_card_ids: v(&["sysml.cases.verification-case"]),
            normative_locators: v(&["SysML 8.3.24.4"]),
            key_facts: v(&["verification", "verify", "requirement"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.analysis-case"),
            question: s("What does an analysis case produce in SysML v2?"),
            expected_card_ids: v(&["sysml.cases.analysis-case"]),
            normative_locators: v(&["SysML 8.3.23.3"]),
            key_facts: v(&["analysis", "case", "result"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.use-case"),
            question: s("What is a use case definition and who participates in it?"),
            expected_card_ids: v(&["sysml.cases.use-case-definition"]),
            normative_locators: v(&["SysML 8.3.25.3"]),
            key_facts: v(&["use case", "actor", "objective"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Views ----------------------------------------------------
        ExplanationEval {
            id: s("exp.view-viewpoint"),
            question: s("What is the relationship between a view and a viewpoint in SysML v2?"),
            expected_card_ids: v(&["sysml.views.view-definition", "sysml.views.viewpoint-definition"]),
            normative_locators: v(&["SysML 8.3.26.7", "SysML 8.3.26.8"]),
            key_facts: v(&["view", "viewpoint", "concern", "render"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.expose"),
            question: s("How does a view expose the model elements it presents?"),
            expected_card_ids: v(&["sysml.views.membership-expose", "sysml.views.namespace-expose"]),
            normative_locators: v(&["SysML 8.3.26.3", "SysML 8.3.26.4"]),
            key_facts: v(&["expose", "view", "member"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Metamodel semantics --------------------------------------
        ExplanationEval {
            id: s("exp.metadata"),
            question: s("How does a metadata definition annotate model elements in SysML v2?"),
            expected_card_ids: v(&["sysml.metadata.metadata-definition", "sysml.metadata.metadata-usage"]),
            normative_locators: v(&["SysML 8.3.27.2", "SysML 8.3.27.3"]),
            key_facts: v(&["metadata", "annotate", "metadata def"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.comment-doc"),
            question: s("What is the difference between a comment and documentation in KerML?"),
            expected_card_ids: v(&["kerml.metadata.comment", "kerml.metadata.documentation"]),
            normative_locators: v(&["KerML 8.3.2.3.4", "KerML 8.3.2.3.5"]),
            key_facts: v(&["comment", "documentation", "note"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Standard library -----------------------------------------
        ExplanationEval {
            id: s("exp.verdict-kind"),
            question: s("What are the possible verdicts of a SysML verification case?"),
            expected_card_ids: v(&["sysml.library.verdict-kind"]),
            normative_locators: v(&["SysML 7.24"]),
            key_facts: v(&["verdict", "pass", "fail"]),
            distinguishes_normative_vs_implementation: false,
        },
        ExplanationEval {
            id: s("exp.requirement-check"),
            question: s("What does the standard-library RequirementCheck evaluate?"),
            expected_card_ids: v(&["sysml.library.requirement-check"]),
            normative_locators: v(&["SysML 7.21"]),
            key_facts: v(&["requirement", "check", "constraint", "subject"]),
            distinguishes_normative_vs_implementation: false,
        },
        // --- Normative-vs-implementation ------------------------------
        ExplanationEval {
            id: s("exp.transition-support-line"),
            question: s("Is a transition usage valid SysML even if sysml-rs does not execute it, and how should that be reported?"),
            expected_card_ids: v(&["sysml.behavior.transition-usage"]),
            normative_locators: v(&["SysML 8.3.18.9"]),
            key_facts: v(&["normative", "valid", "execute", "support"]),
            distinguishes_normative_vs_implementation: true,
        },
    ]
}

fn generation() -> Vec<GenerationEval> {
    vec![
        // --- SysML structure ------------------------------------------
        GenerationEval {
            id: s("gen.part-def-attribute"),
            task: s("Define a part named Compressor with an attribute named ratio."),
            expected_card_ids: v(&["sysml.structure.part-definition", "sysml.structure.attribute-usage"]),
            reference_solution: s("package Gen { part def Compressor { attribute ratio; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.two-parts-connection"),
            task: s("Define a part with two nested parts and a connection between them."),
            expected_card_ids: v(&["sysml.structure.part-usage", "sysml.structure.connection-usage"]),
            reference_solution: s("package Gen { part def Rig { part left; part right; connection link connect left to right; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.enum"),
            task: s("Define an enumeration named Phase with values start and stop."),
            expected_card_ids: v(&["sysml.structure.enumeration-definition"]),
            reference_solution: s("package Gen { enum def Phase { enum start; enum stop; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.port-def"),
            task: s("Define a port named Coolant and a part that has one such port."),
            expected_card_ids: v(&["sysml.structure.port-definition", "sysml.structure.port-usage"]),
            reference_solution: s("package Gen { port def Coolant; part def Chiller { port inlet : Coolant; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.item-def"),
            task: s("Define an item named Fluid."),
            expected_card_ids: v(&["sysml.structure.item-definition"]),
            reference_solution: s("package Gen { item def Fluid; }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.attribute-def-usage"),
            task: s("Define an attribute type named Pressure and a usage of it named p."),
            expected_card_ids: v(&["sysml.structure.attribute-definition", "sysml.structure.attribute-usage"]),
            reference_solution: s("package Gen { attribute def Pressure; attribute p : Pressure; }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.item-usage"),
            task: s("Define an item type Fluid and a usage of it inside a part."),
            expected_card_ids: v(&["sysml.structure.item-definition", "sysml.structure.item-usage"]),
            reference_solution: s("package Gen { item def Fluid; part def Tank { item contents : Fluid; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.reference-usage"),
            task: s("Give a part a reference usage to another part."),
            expected_card_ids: v(&["sysml.structure.reference-usage", "sysml.structure.part-definition"]),
            reference_solution: s("package Gen { part def Engine; part def Car { ref engine : Engine; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.interface-def"),
            task: s("Define a port and an interface definition between two of them."),
            expected_card_ids: v(&["sysml.structure.port-definition", "sysml.structure.interface-definition"]),
            reference_solution: s("package Gen { port def P; interface def I { end p1 : P; end p2 : P; } }"),
            check_resolve: true,
        },
        // --- KerML core / relationships -------------------------------
        GenerationEval {
            id: s("gen.specialization"),
            task: s("Define a base part and a derived part that specializes it."),
            expected_card_ids: v(&["sysml.structure.part-definition", "kerml.structure.specialization"]),
            reference_solution: s("package Gen { part def Base; part def Derived :> Base; }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.subsetting"),
            task: s("Define a part with a whole and a nested part that subsets it."),
            expected_card_ids: v(&["sysml.structure.part-usage", "kerml.structure.subsetting"]),
            reference_solution: s("package Gen { part def Rack { part slots; part firstSlot subsets slots; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.multiplicity"),
            task: s("Define a part with four nested slots using multiplicity."),
            expected_card_ids: v(&["sysml.structure.part-usage", "kerml.structure.multiplicity"]),
            reference_solution: s("package Gen { part def Rack { part slots[4]; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.package-nesting"),
            task: s("Nest one package inside another and define a part in the inner one."),
            expected_card_ids: v(&["kerml.structure.package", "sysml.structure.part-definition"]),
            reference_solution: s("package Outer { package Inner { part def Widget; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.import"),
            task: s("Write two packages where the second imports all members of the first."),
            expected_card_ids: v(&["kerml.structure.import", "sysml.structure.part-definition"]),
            reference_solution: s("package Lib { part def Wheel; } package App { import Lib::*; }"),
            check_resolve: true,
        },
        // --- Expressions ----------------------------------------------
        GenerationEval {
            id: s("gen.literal-integer"),
            task: s("Give an attribute an integer literal default value."),
            expected_card_ids: v(&["sysml.structure.attribute-usage", "kerml.expression.literal-integer"]),
            reference_solution: s("package Gen { part def P { attribute count = 5; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.literal-boolean"),
            task: s("Give an attribute a boolean literal default value."),
            expected_card_ids: v(&["sysml.structure.attribute-usage", "kerml.expression.literal-boolean"]),
            reference_solution: s("package Gen { part def P { attribute ok = true; } }"),
            check_resolve: true,
        },
        // --- Action / behavior ----------------------------------------
        GenerationEval {
            id: s("gen.action-def"),
            task: s("Define an action named Pump."),
            expected_card_ids: v(&["sysml.behavior.action-definition"]),
            reference_solution: s("package Gen { action def Pump; }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.action-nested"),
            task: s("Define an action with two nested sub-actions."),
            expected_card_ids: v(&["sysml.behavior.action-definition", "sysml.behavior.action-usage"]),
            reference_solution: s("package Gen { action def Process { action step1; action step2; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.calc-def"),
            task: s("Define a calculation that adds two inputs and returns the sum."),
            expected_card_ids: v(&["sysml.behavior.calculation-definition"]),
            reference_solution: s("package Gen { calc def Add { in a; in b; return sum = a + b; } }"),
            check_resolve: false,
        },
        // --- States ---------------------------------------------------
        GenerationEval {
            id: s("gen.state-machine"),
            task: s("Model a state machine with an idle state, a running state, and a transition between them."),
            expected_card_ids: v(&["sysml.behavior.state-definition", "sysml.behavior.transition-usage"]),
            reference_solution: s("package Gen { state def Motor { state idle; state running; transition first idle then running; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.state-entry"),
            task: s("Define a state with an entry action."),
            expected_card_ids: v(&["sysml.behavior.state-definition", "sysml.behavior.state-usage"]),
            reference_solution: s("package Gen { state def Mode { entry action init; state active; } }"),
            check_resolve: true,
        },
        // --- Requirements / cases -------------------------------------
        GenerationEval {
            id: s("gen.requirement-subject"),
            task: s("Write a requirement definition named Reliability with a subject."),
            expected_card_ids: v(&["sysml.requirements.requirement-definition", "sysml.requirements.subject"]),
            reference_solution: s("package Gen { requirement def Reliability { subject unit; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.requirement-usage"),
            task: s("Define a requirement type and a usage of it."),
            expected_card_ids: v(&["sysml.requirements.requirement-definition", "sysml.requirements.requirement-usage"]),
            reference_solution: s("package Gen { requirement def Safe { subject s; } requirement r : Safe; }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.constraint-def"),
            task: s("Define a constraint definition with a boolean body over an input."),
            expected_card_ids: v(&["sysml.requirements.constraint-definition"]),
            reference_solution: s("package Gen { constraint def Positive { in x; x > 0 } }"),
            check_resolve: false,
        },
        GenerationEval {
            id: s("gen.verification-case"),
            task: s("Define a verification case definition with a subject."),
            expected_card_ids: v(&["sysml.cases.verification-case-definition"]),
            reference_solution: s("package Gen { verification def VC { subject unit; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.use-case"),
            task: s("Define a use case definition with a subject."),
            expected_card_ids: v(&["sysml.cases.use-case-definition"]),
            reference_solution: s("package Gen { use case def UC { subject sys; } }"),
            check_resolve: true,
        },
        // --- Views ----------------------------------------------------
        GenerationEval {
            id: s("gen.view-def"),
            task: s("Define a view definition and a viewpoint definition."),
            expected_card_ids: v(&["sysml.views.view-definition", "sysml.views.viewpoint-definition"]),
            reference_solution: s("package Gen { viewpoint def VP; view def V; }"),
            check_resolve: true,
        },
        // --- Metadata -------------------------------------------------
        GenerationEval {
            id: s("gen.metadata-def"),
            task: s("Define a metadata definition."),
            expected_card_ids: v(&["sysml.metadata.metadata-definition"]),
            reference_solution: s("package Gen { metadata def Origin { attribute source; } }"),
            check_resolve: true,
        },
        GenerationEval {
            id: s("gen.documentation"),
            task: s("Attach documentation to a part definition."),
            expected_card_ids: v(&["kerml.metadata.documentation", "sysml.structure.part-definition"]),
            reference_solution: s("package Gen { part def Widget { doc /* the primary widget */ } }"),
            check_resolve: true,
        },
    ]
}

fn repair() -> Vec<RepairEval> {
    vec![
        // --- parse phase (structurally guaranteed failures) ---------------
        RepairEval {
            id: s("rep.unclosed-body"),
            broken_source: s("package Rep { part def Tank { attribute volume; "),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { part def Tank { attribute volume; } }"),
            expected_card_ids: v(&["sysml.structure.part-definition", "sysml.structure.attribute-usage"]),
        },
        RepairEval {
            id: s("rep.transition-missing-target"),
            broken_source: s("package Rep { state def SM { state a; state b; transition first a then ; } }"),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { state def SM { state a; state b; transition first a then b; } }"),
            expected_card_ids: v(&["sysml.behavior.transition-usage"]),
        },
        RepairEval {
            id: s("rep.connection-missing-to"),
            broken_source: s("package Rep { part def Rig { part x; part y; connection c connect x y; } }"),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { part def Rig { part x; part y; connection c connect x to y; } }"),
            expected_card_ids: v(&["sysml.structure.connection-usage"]),
        },
        // --- resolve phase (unresolved references) ------------------------
        RepairEval {
            id: s("rep.unresolved-type"),
            broken_source: s("package Rep { part pump : Missing; }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { part def Pump; part pump : Pump; }"),
            expected_card_ids: v(&["sysml.structure.part-usage", "sysml.structure.part-definition"]),
        },
        RepairEval {
            id: s("rep.unresolved-attribute-type"),
            broken_source: s("package Rep { attribute speed : Velocity; }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { attribute def Velocity; attribute speed : Velocity; }"),
            expected_card_ids: v(&["sysml.structure.attribute-usage", "sysml.structure.attribute-definition"]),
        },
        RepairEval {
            id: s("rep.unresolved-specialization"),
            broken_source: s("package Rep { item def Coolant :> BaseFluid; }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { item def BaseFluid; item def Coolant :> BaseFluid; }"),
            expected_card_ids: v(&["sysml.structure.item-definition"]),
        },
        // --- validate phase (specific S0xx validators) --------------------
        RepairEval {
            id: s("rep.duplicate-member-name"),
            broken_source: s("package Reg { attribute def Voltage; attribute def Voltage; }"),
            expected_phase: s("validate"),
            expected_diagnostic_category: s("relationship"),
            diagnostic_code: Some(s("S001")),
            fixed_source: s("package Reg { attribute def Voltage; attribute def Current; }"),
            expected_card_ids: v(&["kerml.structure.namespace"]),
        },
        RepairEval {
            id: s("rep.two-subjects"),
            broken_source: s("package Rep { requirement def Safety { subject sysA; subject sysB; } }"),
            expected_phase: s("validate"),
            expected_diagnostic_category: s("relationship"),
            diagnostic_code: Some(s("S060")),
            fixed_source: s("package Rep { requirement def Safety { subject sysA; } }"),
            expected_card_ids: v(&["sysml.requirements.requirement-definition"]),
        },
        RepairEval {
            id: s("rep.two-entry-actions"),
            broken_source: s("package Rep { state def Mode { entry action init; entry action boot; } }"),
            expected_phase: s("validate"),
            expected_diagnostic_category: s("relationship"),
            diagnostic_code: Some(s("S068")),
            fixed_source: s("package Rep { state def Mode { entry action init; } }"),
            expected_card_ids: v(&["sysml.behavior.state-usage"]),
        },
        // --- more parse-phase (structurally guaranteed failures) ----------
        RepairEval {
            id: s("rep.state-unclosed"),
            broken_source: s("package Rep { state def SM { state a; state b; "),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { state def SM { state a; state b; } }"),
            expected_card_ids: v(&["sysml.behavior.state-definition"]),
        },
        RepairEval {
            id: s("rep.enum-unclosed"),
            broken_source: s("package Rep { enum def Color { enum red; enum green; "),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { enum def Color { enum red; enum green; } }"),
            expected_card_ids: v(&["sysml.structure.enumeration-definition"]),
        },
        RepairEval {
            id: s("rep.package-unclosed"),
            broken_source: s("package Rep { part def Widget; "),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { part def Widget; }"),
            expected_card_ids: v(&["kerml.structure.package", "sysml.structure.part-definition"]),
        },
        RepairEval {
            id: s("rep.action-unclosed"),
            broken_source: s("package Rep { action def Process { action step1; "),
            expected_phase: s("parse"),
            expected_diagnostic_category: s("keyword-shape"),
            diagnostic_code: None,
            fixed_source: s("package Rep { action def Process { action step1; } }"),
            expected_card_ids: v(&["sysml.behavior.action-definition"]),
        },
        // --- more resolve-phase (unresolved references) -------------------
        RepairEval {
            id: s("rep.unresolved-port-type"),
            broken_source: s("package Rep { part def Chiller { port inlet : Coolant; } }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { port def Coolant; part def Chiller { port inlet : Coolant; } }"),
            expected_card_ids: v(&["sysml.structure.port-usage", "sysml.structure.port-definition"]),
        },
        RepairEval {
            id: s("rep.unresolved-item-type"),
            broken_source: s("package Rep { part def Tank { item contents : Fluid; } }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { item def Fluid; part def Tank { item contents : Fluid; } }"),
            expected_card_ids: v(&["sysml.structure.item-usage", "sysml.structure.item-definition"]),
        },
        RepairEval {
            id: s("rep.unresolved-requirement-type"),
            broken_source: s("package Rep { requirement r : Reliability; }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { requirement def Reliability { subject s; } requirement r : Reliability; }"),
            expected_card_ids: v(&["sysml.requirements.requirement-usage", "sysml.requirements.requirement-definition"]),
        },
        RepairEval {
            id: s("rep.unresolved-subsetting"),
            broken_source: s("package Rep { part def Rack { part firstSlot subsets slots; } }"),
            expected_phase: s("resolve"),
            expected_diagnostic_category: s("reference"),
            diagnostic_code: None,
            fixed_source: s("package Rep { part def Rack { part slots; part firstSlot subsets slots; } }"),
            expected_card_ids: v(&["sysml.structure.part-usage", "kerml.structure.subsetting"]),
        },
        // --- more validate-phase (specific S0xx validators) ---------------
        RepairEval {
            id: s("rep.duplicate-part-name"),
            broken_source: s("package Reg { part def Widget; part def Widget; }"),
            expected_phase: s("validate"),
            expected_diagnostic_category: s("relationship"),
            diagnostic_code: Some(s("S001")),
            fixed_source: s("package Reg { part def Widget; part def Gadget; }"),
            expected_card_ids: v(&["kerml.structure.namespace"]),
        },
        RepairEval {
            id: s("rep.two-subjects-usage"),
            broken_source: s("package Rep { requirement safety { subject sysA; subject sysB; } }"),
            expected_phase: s("validate"),
            expected_diagnostic_category: s("relationship"),
            diagnostic_code: Some(s("S061")),
            fixed_source: s("package Rep { requirement safety { subject sysA; } }"),
            expected_card_ids: v(&["sysml.requirements.requirement-usage"]),
        },
    ]
}

/// Support-discrimination items. `expected_support` is asserted equal to the
/// card's live support axis by the gate, so these stay honest.
fn support_discrimination() -> Vec<SupportDiscriminationEval> {
    vec![
        SupportDiscriminationEval {
            id: s("sd.transition-execute"),
            question: s("A transition usage is valid SysML — does sysml-rs actually execute transition-usage models today?"),
            concept_card_id: s("sysml.behavior.transition-usage"),
            axis: s("execute"),
            expected_support: s("unknown"),
            answer_key: s("Transition usage is normative, valid SysML (grammar rule TransitionUsage, clause 8.3.18.9). Its sysml-rs execute-axis support is 'unknown' — the pack records no executable evidence — so language validity must not be reported as tool-executable."),
        },
        SupportDiscriminationEval {
            id: s("sd.part-def-resolve"),
            question: s("Does sysml-rs resolve references for part definitions, or only accept them syntactically?"),
            concept_card_id: s("sysml.structure.part-definition"),
            axis: s("resolve"),
            expected_support: s("validated"),
            answer_key: s("Part definition has 'validated' resolve-axis support in sysml-rs, backed by an executable evidence case — resolution is confirmed, not merely parsed."),
        },
        SupportDiscriminationEval {
            id: s("sd.connection-validate"),
            question: s("Does sysml-rs enforce the well-formedness rules on connection usages?"),
            concept_card_id: s("sysml.structure.connection-usage"),
            axis: s("validate"),
            expected_support: s("validated"),
            answer_key: s("Connection usage has 'validated' validate-axis support — the S106 connector-owned-by-type check fires, so sysml-rs enforces its well-formedness rule."),
        },
        SupportDiscriminationEval {
            id: s("sd.additive-operator-parse"),
            question: s("The additive operator is part of the SysML expression grammar — has sysml-rs demonstrated it parses one?"),
            concept_card_id: s("kerml.expression.additive-operator"),
            axis: s("parse"),
            expected_support: s("unknown"),
            answer_key: s("The additive operator is normative KerML expression syntax, but its parse-axis support in the pack is 'unknown' (no per-operator example carries executable evidence). Normative existence does not by itself establish demonstrated implementation support."),
        },
        SupportDiscriminationEval {
            id: s("sd.verdict-kind-parse"),
            question: s("VerdictKind is defined in the standard library — does the pack claim sysml-rs parse support for it as a syntactic concept?"),
            concept_card_id: s("sysml.library.verdict-kind"),
            axis: s("parse"),
            expected_support: s("unknown"),
            answer_key: s("VerdictKind is a library-defined semantic construct, not a syntactic production, so its parse-axis support is 'unknown' — its meaning is the normative library definition, not a grammar rule."),
        },
        SupportDiscriminationEval {
            id: s("sd.requirement-def-validate"),
            question: s("Does sysml-rs actually check that a requirement definition has at most one subject?"),
            concept_card_id: s("sysml.requirements.requirement-definition"),
            axis: s("validate"),
            expected_support: s("validated"),
            answer_key: s("Yes — requirement definition has 'validated' validate-axis support; the S060 single-subject check fires as an executable gate."),
        },
        SupportDiscriminationEval {
            id: s("sd.action-usage-execute"),
            question: s("Action usage parses and lowers in sysml-rs — does that mean it executes?"),
            concept_card_id: s("sysml.behavior.action-usage"),
            axis: s("execute"),
            expected_support: s("unknown"),
            answer_key: s("No. Action usage has validated parse/lower support but 'unknown' execute-axis support. Parse and lower success is not evidence of runtime execution; the axes are reported independently."),
        },
        SupportDiscriminationEval {
            id: s("sd.part-usage-format"),
            question: s("Can sysml-rs format (pretty-print) a part usage back to canonical text?"),
            concept_card_id: s("sysml.structure.part-usage"),
            axis: s("format"),
            expected_support: s("unknown"),
            answer_key: s("The format-axis support for part usage is 'unknown' in the pack — no formatting evidence is recorded — so a valid part usage cannot be assumed to round-trip through a formatter."),
        },
        SupportDiscriminationEval {
            id: s("sd.part-def-parse"),
            question: s("Has sysml-rs demonstrated that it parses a part definition?"),
            concept_card_id: s("sysml.structure.part-definition"),
            axis: s("parse"),
            expected_support: s("validated"),
            answer_key: s("Yes — part definition has 'validated' parse-axis support, backed by an executable parse-evidence case; the parser demonstrably accepts it."),
        },
        SupportDiscriminationEval {
            id: s("sd.transition-resolve"),
            question: s("A transition usage parses in sysml-rs — does that mean its references resolve too?"),
            concept_card_id: s("sysml.behavior.transition-usage"),
            axis: s("resolve"),
            expected_support: s("unknown"),
            answer_key: s("No. Transition usage has 'validated' parse and lower support but 'unknown' resolve-axis support — parse success does not imply resolution is demonstrated; the axes are reported independently."),
        },
        SupportDiscriminationEval {
            id: s("sd.literal-integer-resolve"),
            question: s("Is the resolve-axis support for an integer literal the same as its parse-axis support in sysml-rs?"),
            concept_card_id: s("kerml.expression.literal-integer"),
            axis: s("resolve"),
            expected_support: s("unknown"),
            answer_key: s("No — an integer literal has 'validated' parse support but 'unknown' resolve support; a literal carries no reference to resolve, and the pack records no resolve evidence, so the axes differ."),
        },
        SupportDiscriminationEval {
            id: s("sd.additive-operator-lower"),
            question: s("Does the pack claim sysml-rs lowers the additive operator, given it is normative KerML syntax?"),
            concept_card_id: s("kerml.expression.additive-operator"),
            axis: s("lower"),
            expected_support: s("unknown"),
            answer_key: s("The additive operator is normative KerML expression syntax, but its lower-axis support is 'unknown' — normative existence is not demonstrated tool support. Do not report language validity as tool capability."),
        },
        SupportDiscriminationEval {
            id: s("sd.state-usage-validate"),
            question: s("Does sysml-rs enforce a well-formedness rule on state usages?"),
            concept_card_id: s("sysml.behavior.state-usage"),
            axis: s("validate"),
            expected_support: s("validated"),
            answer_key: s("Yes — state usage has 'validated' validate-axis support; a state well-formedness validator fires as an executable gate."),
        },
        SupportDiscriminationEval {
            id: s("sd.flow-connection-validate"),
            question: s("Does sysml-rs check the well-formedness of a flow connection?"),
            concept_card_id: s("sysml.structure.flow-connection"),
            axis: s("validate"),
            expected_support: s("validated"),
            answer_key: s("Yes — flow connection has 'validated' validate-axis support, backed by an executable validation case; the rule is enforced, not merely parsed."),
        },
        SupportDiscriminationEval {
            id: s("sd.namespace-validate"),
            question: s("Does sysml-rs enforce that members of a namespace have unique names?"),
            concept_card_id: s("kerml.structure.namespace"),
            axis: s("validate"),
            expected_support: s("validated"),
            answer_key: s("Yes — namespace has 'validated' validate-axis support; the S001 unique-owned-member-names check fires, so sysml-rs enforces this rule."),
        },
        SupportDiscriminationEval {
            id: s("sd.enumeration-resolve"),
            question: s("An enumeration definition parses in sysml-rs — is its resolve-axis support also validated?"),
            concept_card_id: s("sysml.structure.enumeration-definition"),
            axis: s("resolve"),
            expected_support: s("validated"),
            answer_key: s("Yes — enumeration definition now has 'validated' resolve-axis support: enumerated values resolve (a qualified reference like `Color::red` resolves to the distinct EnumerationUsage, and each value is typed by its owning enum def), so reference resolution is confirmed, not merely parsing."),
        },
        SupportDiscriminationEval {
            id: s("sd.decision-node-resolve"),
            question: s("Does sysml-rs resolve references for a decision node, not just parse it?"),
            concept_card_id: s("sysml.behavior.decision-node"),
            axis: s("resolve"),
            expected_support: s("validated"),
            answer_key: s("Yes — decision node has 'validated' resolve-axis support, backed by an executable evidence case; resolution is confirmed."),
        },
        SupportDiscriminationEval {
            id: s("sd.calculation-def-execute"),
            question: s("A calculation definition resolves in sysml-rs — does the pack claim it executes?"),
            concept_card_id: s("sysml.behavior.calculation-definition"),
            axis: s("execute"),
            expected_support: s("unknown"),
            answer_key: s("No. Calculation definition has 'validated' parse/lower/resolve support but 'unknown' execute-axis support — resolution success is not evidence of runtime evaluation."),
        },
        SupportDiscriminationEval {
            id: s("sd.item-def-format"),
            question: s("Can sysml-rs pretty-print an item definition back to canonical text?"),
            concept_card_id: s("sysml.structure.item-definition"),
            axis: s("format"),
            expected_support: s("unknown"),
            answer_key: s("The format-axis support for item definition is 'unknown' — no formatting evidence is recorded — so round-tripping through a formatter cannot be assumed."),
        },
        SupportDiscriminationEval {
            id: s("sd.verification-case-lsp"),
            question: s("Does the pack record LSP support for a verification case in sysml-rs?"),
            concept_card_id: s("sysml.cases.verification-case"),
            axis: s("lsp"),
            expected_support: s("unknown"),
            answer_key: s("No — verification case has 'validated' parse/lower/resolve support but 'unknown' lsp-axis support; editor-feature support is a distinct axis with no recorded evidence."),
        },
        SupportDiscriminationEval {
            id: s("sd.validation-card-parse"),
            question: s("The S001 unique-member-names validation card — does it have parse-axis support in the pack?"),
            concept_card_id: s("kerml.validation.unique-owned-member-names"),
            axis: s("parse"),
            expected_support: s("unknown"),
            answer_key: s("The unique-owned-member-names card describes a validation obligation, not a syntactic production, so its parse-axis support is 'unknown' — it is a semantic rule, not a grammar rule to parse."),
        },
    ]
}

/// Serialize records to JSONL sorted by id (deterministic).
fn to_jsonl<T: Serialize>(
    records: &[T],
    id_of: impl Fn(&T) -> &str,
) -> Result<String, LpError> {
    let mut rows: Vec<(&str, String)> = Vec::new();
    for r in records {
        let json =
            serde_json::to_string(r).map_err(|e| LpError::Other(format!("eval serialize: {e}")))?;
        rows.push((id_of(r), json));
    }
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::new();
    for (_, json) in rows {
        out.push_str(&json);
        out.push('\n');
    }
    Ok(out)
}

/// Build all four eval datasets as `(name, jsonl)` pairs.
pub fn export_evals() -> Result<Vec<(&'static str, String)>, LpError> {
    Ok(vec![
        ("explanation", to_jsonl(&explanation(), |r| &r.id)?),
        ("generation", to_jsonl(&generation(), |r| &r.id)?),
        ("repair", to_jsonl(&repair(), |r| &r.id)?),
        ("support-discrimination", to_jsonl(&support_discrimination(), |r| &r.id)?),
    ])
}
