//! Concept classification, ID minting, and denominator records.
//! For the vertical slice this classifies the one rule and folds its
//! structural helpers; the full 725->650 pipeline is packet 12.

use serde::Serialize;

/// Closed classification vocabulary (matches
/// `denominator-record.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    UserFacing,
    StructuralHelper,
    LexicalHelper,
    Alias,
    AbstractOnly,
    Expression,
    Operator,
    SemanticOnly,
    LibraryDefined,
    Excluded,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Classification::UserFacing => "user-facing",
            Classification::StructuralHelper => "structural-helper",
            Classification::LexicalHelper => "lexical-helper",
            Classification::Alias => "alias",
            Classification::AbstractOnly => "abstract-only",
            Classification::Expression => "expression",
            Classification::Operator => "operator",
            Classification::SemanticOnly => "semantic-only",
            Classification::LibraryDefined => "library-defined",
            Classification::Excluded => "excluded",
        }
    }

    /// Whether this classification mints a user-facing card.
    pub fn is_card_bearing(self) -> bool {
        matches!(
            self,
            Classification::UserFacing
                | Classification::Expression
                | Classification::Operator
                | Classification::SemanticOnly
                | Classification::LibraryDefined
        )
    }
}

/// Names given an explicit reviewed classification that the coarse suffix
/// heuristic gets wrong, each with the rationale published in the denominator
/// report (reclassifications on deliberate review are recorded,
/// not silently forced into cards). These are the cross-grammar *divergent*
/// names the coarse heuristic marked user-facing but that a grammar-grounded
/// review finds to be concrete-syntax fragments, `Owned*`/root wrappers, or
/// sub-productions already covered by a concept card — so they fold into their
/// parent card's `rule_dependencies` rather than minting a card of their own.
/// FeatureTyping is deliberately absent: it returns a distinct `FeatureTyping`
/// element kind and is the canonical §6.2 split pair (kerml/sysml cards).
pub const REVIEWED_RECLASSIFICATIONS: &[(&str, Classification, &str)] = &[
    ("ConnectorEnd", Classification::StructuralHelper, "a connector endpoint fragment (kerml returns Feature, sysml ReferenceUsage); modellers write `connect a to b`, the ends are implicit — folds under connection-usage"),
    ("FeatureSpecialization", Classification::StructuralHelper, "the specialization clause of a feature declaration (returns Feature); grammar plumbing collecting typing/subsetting/redefinition, not a written concept"),
    ("OwnedFeatureTyping", Classification::StructuralHelper, "an Owned* wrapper producing a FeatureTyping; §3.2 aliases Owned* siblings — folds under feature-typing"),
    ("OwnedMultiplicity", Classification::StructuralHelper, "an OwningMembership wrapper for a multiplicity; membership plumbing — folds under multiplicity"),
    ("PayloadFeature", Classification::StructuralHelper, "the payload fragment of accept/send action nodes; folds under the action-node concepts"),
    ("PrefixMetadataAnnotation", Classification::StructuralHelper, "the `#Meta` prefix-annotation fragment (returns Annotation); folds under metadata-usage"),
    ("RootNamespace", Classification::StructuralHelper, "the implicit file-root namespace container, not a modeller-written construct (predecessor-flagged candidate)"),
    ("Typings", Classification::StructuralHelper, "a fragment collecting one-or-more typings (returns Feature); the plural syntax plumbing behind feature-typing"),
    ("MultiplicityRange", Classification::StructuralHelper, "the `[m..n]` range sub-production of multiplicity; folds under the multiplicity concept card"),
    ("TypedBy", Classification::StructuralHelper, "the `:`/`typed by` typing-clause fragment (returns Feature) that produces a FeatureTyping relationship — folds under feature-typing"),
    ("Subsets", Classification::StructuralHelper, "the `:>`/`subsets` subsetting-clause fragment (returns Feature) producing a Subsetting relationship — folds under specialization"),
    ("Redefines", Classification::StructuralHelper, "the `:>>`/`redefines` redefinition-clause fragment (returns Feature) producing a Redefinition relationship — folds under specialization"),
    // KerML-relationships family: the `Owned*` owned-relationship wrappers
    // and plural collectors are grammar plumbing for the base relationship
    // concepts (conjugation/disjoining/subsetting/redefinition/…), not
    // separately-written constructs — same character as the Owned*/plural forms
    // already reclassified above (OwnedFeatureTyping, OwnedMultiplicity, Typings).
    ("OwnedConjugation", Classification::StructuralHelper, "an Owned* wrapper producing a Conjugation; folds under the conjugation relationship concept"),
    ("OwnedDisjoining", Classification::StructuralHelper, "an Owned* wrapper producing a Disjoining; folds under the disjoining relationship concept"),
    ("OwnedSubsetting", Classification::StructuralHelper, "an Owned* wrapper producing a Subsetting; folds under kerml.structure.subsetting"),
    ("OwnedRedefinition", Classification::StructuralHelper, "an Owned* wrapper producing a Redefinition; folds under kerml.structure.redefinition"),
    ("OwnedFeatureInverting", Classification::StructuralHelper, "an Owned* wrapper producing a FeatureInverting; folds under the feature-inversion relationship concept"),
    ("OwnedTypeFeaturing", Classification::StructuralHelper, "an Owned* wrapper producing a TypeFeaturing; folds under the type-featuring relationship concept"),
    ("OwnedSpecialization", Classification::StructuralHelper, "an Owned* wrapper producing a Specialization; folds under kerml.structure.specialization"),
    ("Ownedsubclassification", Classification::StructuralHelper, "an Owned* wrapper producing a Subclassification; folds under kerml.structure.specialization"),
    ("OwnedSubclassification", Classification::StructuralHelper, "an Owned* wrapper producing a Subclassification (SysML grammar); folds under kerml.structure.specialization"),
    ("OwnedReferenceSubsetting", Classification::StructuralHelper, "an Owned* wrapper producing a reference-subsetting (`::>`); folds under kerml.structure.subsetting"),
    ("OwnedCrossSubsetting", Classification::StructuralHelper, "an Owned* wrapper producing a cross-subsetting fragment of the flow-crossing syntax; folds under the crossing concept"),
    ("OwnedCrossingFeature", Classification::StructuralHelper, "an Owned* crossing-feature fragment of the flow-crossing syntax; folds under the crossing concept"),
    ("OwnedCrossingMultiplicity", Classification::StructuralHelper, "an Owned* crossing-multiplicity fragment; folds under the crossing concept"),
    ("OwnedCrossFeature", Classification::StructuralHelper, "an Owned* crossing-feature fragment (SysML grammar); folds under the crossing concept"),
    ("OwnedCrossMultiplicity", Classification::StructuralHelper, "an Owned* crossing-multiplicity fragment (SysML grammar); folds under the crossing concept"),
    ("OwnedMultiplicityRange", Classification::StructuralHelper, "an Owned* wrapper for a multiplicity range; folds under the multiplicity concept card"),
    ("Subsettings", Classification::StructuralHelper, "a fragment collecting one-or-more subsettings; the plural syntax plumbing behind subsetting"),
    ("Redefinitions", Classification::StructuralHelper, "a fragment collecting one-or-more redefinitions; the plural syntax plumbing behind redefinition"),
    // Uncarded-tail closure: the long tail of grammar-plumbing rule
    // names the coarse suffix heuristic left `user-facing`+`uncarded`, reviewed
    // against each rule's xtext `returns` metaclass. Every entry here returns a
    // GENERIC base type, is an xtext `fragment`/`enum` production, an abstract
    // base, or a keyword form that lowers to an already-carded kind — none is a
    // separately-written construct, so each folds into its carded parent rather
    // than lowering `user_facing_syntax_coverage` as a false gap. Names that DO
    // return their own distinct metaclass are carded; real
    // parse/lowering gaps stay `uncarded` with a known-gap reference (Wave C).
    ("ActionBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ActionBodyParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ActionNode", Classification::AbstractOnly, "an abstract base whose concrete written forms are carded: control/action nodes (fork, join, merge, decision, accept, send) and expose forms (namespace-expose, membership-expose)"),
    ("AdditiveExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("AndExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("Annotation", Classification::AbstractOnly, "the abstract annotating-element relationship base; its concrete written forms — comment, documentation, metadata — are carded, and the base is never written directly"),
    ("Argument", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ArgumentExpression", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ArgumentExpressionValue", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ArgumentList", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ArgumentValue", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("BaseExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("BodyExpression", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("BodyParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("BooleanExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("BooleanValue", Classification::LexicalHelper, "a terminal value production (Ecore Boolean/Real datatype token) consumed by literal expressions; a lexical token, not a construct"),
    ("CalculationBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("CaseBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ChangeTriggerKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("ClassificationExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("ConditionalExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("ConjugatedPortTyping", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ConjugatedQualifiedName", Classification::LexicalHelper, "a name / qualified-name lexical production of the reference grammar; it forms the textual reference token consumed by every concept that names a target — not a construct itself"),
    ("ConstructorResult", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ControlNode", Classification::AbstractOnly, "an abstract base whose concrete written forms are carded: control/action nodes (fork, join, merge, decision, accept, send) and expose forms (namespace-expose, membership-expose)"),
    ("Crosses", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("Crossings", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("DefaultInterfaceEnd", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("DefaultReferenceUsage", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("Definition", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("DefinitionBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("DoActionKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("EffectFeatureKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("EmptyActionUsage", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("EmptyFeature", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("EmptyMultiplicity", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("EmptySourceEnd", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("EmptyTargetEnd", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("EmptyUsage", Classification::StructuralHelper, "an implicit/empty/default grammar production that synthesizes an unwritten usage or endpoint; it is inserted by the grammar, never written by a modeller — folds into its parent concept"),
    ("EntryActionKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("EqualityExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("EqualityExpressionReference", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("ExitActionKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("ExponentiationExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("Expose", Classification::AbstractOnly, "an abstract base whose concrete written forms are carded: control/action nodes (fork, join, merge, decision, accept, send) and expose forms (namespace-expose, membership-expose)"),
    ("ExposeVisibilityKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("Expression", Classification::AbstractOnly, "an abstract expression base type; concrete expressions (literals, operators, references) carry their own cards — the base is never written directly"),
    ("ExtendedDefinition", Classification::StructuralHelper, "the prefix-metadata `#Meta` form of a definition/usage; it returns a generic Definition/Usage (the metadata is a PrefixMetadata annotation, carded under metadata-usage) — folds into the definition/usage concept"),
    ("ExtendedUsage", Classification::StructuralHelper, "the prefix-metadata `#Meta` form of a definition/usage; it returns a generic Definition/Usage (the metadata is a PrefixMetadata annotation, carded under metadata-usage) — folds into the definition/usage concept"),
    ("ExtentExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("Feature", Classification::AbstractOnly, "the abstract KerML Feature base; the concrete written forms (`feature x`, attribute/part/ref usages) lower to their own carded kinds — the base type is never written directly"),
    ("FeatureBinding", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("FeatureChain", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("FeatureDirection", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("FeatureType", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("FilterPackageImport", Classification::StructuralHelper, "a filtered-import form of the filter-package syntax; it folds into the filter-package concept (carded) whose element-filter it applies to an import"),
    ("FilterPackageMemberVisibility", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("FilterPackageMembershipImport", Classification::StructuralHelper, "a filtered-import form of the filter-package syntax; it folds into the filter-package concept (carded) whose element-filter it applies to an import"),
    ("FilterPackageNamespaceImport", Classification::StructuralHelper, "a filtered-import form of the filter-package syntax; it folds into the filter-package concept (carded) whose element-filter it applies to an import"),
    ("Flow", Classification::StructuralHelper, "a keyword form that lowers to an already-carded usage kind (step/flow/perform/target-transition → Step/FlowUsage/ActionUsage/TransitionUsage) — folds into that concept's card"),
    ("FlowEnd", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("FlowEndSubsetting", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("FlowFeature", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("FlowRedefinition", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("FramedConcernKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("FunctionReference", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("FunctionReferenceExpression", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("GlobalQualification", Classification::LexicalHelper, "a name / qualified-name lexical production of the reference grammar; it forms the textual reference token consumed by every concept that names a target — not a construct itself"),
    ("GuardFeatureKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("Identification", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ImpliesExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("ImpliesExpressionReference", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("ImportedMembership", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ImportedNamespace", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("InterfaceBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("InterfaceEnd", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("LiteralExpression", Classification::AbstractOnly, "an abstract expression base type; concrete expressions (literals, operators, references) carry their own cards — the base is never written directly"),
    ("MetadataBodyFeature", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("MetadataBodyUsage", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("MetadataFeature", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("MetadataReference", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("MetadataTyping", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("MultiplicativeExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("MultiplicityBounds", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("MultiplicitySourceEnd", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("MultiplicitySubset", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("Name", Classification::LexicalHelper, "a name / qualified-name lexical production of the reference grammar; it forms the textual reference token consumed by every concept that names a target — not a construct itself"),
    ("NamedArgument", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("NamedArgumentList", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("NodeParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("NullCoalescingExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("OrExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("OrExpressionReference", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("OwnedAnnotation", Classification::AbstractOnly, "the abstract annotating-element relationship base; its concrete written forms — comment, documentation, metadata — are carded, and the base is never written directly"),
    ("OwnedExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("OwnedExpressionReference", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("OwnedFeatureChain", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("OwnedFeatureChaining", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("ParameterRedefinition", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("Payload", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("PayloadParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("PerformedActionUsage", Classification::StructuralHelper, "a keyword form that lowers to an already-carded usage kind (step/flow/perform/target-transition → Step/FlowUsage/ActionUsage/TransitionUsage) — folds into that concept's card"),
    ("PortionKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("PositionalArgumentList", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("PrefixMetadataFeature", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("PrefixMetadataUsage", Classification::StructuralHelper, "a metadata plumbing production (prefix-metadata clause, metadata-body member, metadata typing/reference) that folds into the carded metadata-usage / metadata-definition / metadata-access concepts"),
    ("PrimaryExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("Qualification", Classification::LexicalHelper, "a name / qualified-name lexical production of the reference grammar; it forms the textual reference token consumed by every concept that names a target — not a construct itself"),
    ("QualifiedName", Classification::LexicalHelper, "a name / qualified-name lexical production of the reference grammar; it forms the textual reference token consumed by every concept that names a target — not a construct itself"),
    ("RangeExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("RealValue", Classification::LexicalHelper, "a terminal value production (Ecore Boolean/Real datatype token) consumed by literal expressions; a lexical token, not a construct"),
    ("ReferenceTyping", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("References", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("RelationalExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("RequirementBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("RequirementConstraintKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("RequirementVerificationKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("SatisfactionFeatureValue", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("SatisfactionParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("SatisfactionReferenceExpression", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("SelfReferenceExpression", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("SequenceExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("StateBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("Step", Classification::StructuralHelper, "a keyword form that lowers to an already-carded usage kind (step/flow/perform/target-transition → Step/FlowUsage/ActionUsage/TransitionUsage) — folds into that concept's card"),
    ("Subclassification", Classification::StructuralHelper, "the class-specialization (`:>` on a classifier) production; it produces a Subclassification already covered by the kerml.structure.specialization concept card"),
    ("SuccessionFlow", Classification::StructuralHelper, "the KerML type-level succession-flow production; it lowers to the SuccessionFlowUsage kind carded as sysml.behavior.succession-flow-usage — folds into that concept"),
    ("TargetBinding", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("TargetExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("TargetFeature", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("TargetParameter", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("TargetTransitionUsage", Classification::StructuralHelper, "a keyword form that lowers to an already-carded usage kind (step/flow/perform/target-transition → Step/FlowUsage/ActionUsage/TransitionUsage) — folds into that concept's card"),
    ("TimeTriggerKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("TriggerExpression", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
    ("TriggerFeatureKind", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("TriggerFeatureValue", Classification::StructuralHelper, "a connector-end / binding / multiplicity sub-production returning a generic endpoint or feature-value; the written concept is the flow/interface/binding/multiplicity it belongs to, which is carded"),
    ("TypeReference", Classification::StructuralHelper, "an argument/parameter/feature-clause plumbing production returning a generic Feature/ReferenceUsage/FeatureValue/typing; the written concept is the enclosing invocation/usage/typing, which is carded — this is its sub-clause"),
    ("UnaryExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("Usage", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("UsageCompletion", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("VariantReference", Classification::StructuralHelper, "the `variant` member reference inside a variation; it lowers to a plain ReferenceUsage (carded as reference-usage), the variation semantics riding on the enclosing VariantMembership — folds into reference-usage"),
    ("ViewBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ViewDefinitionBodyItem", Classification::StructuralHelper, "an xtext grammar fragment (a reusable body/member/collector sub-production spliced into a parent rule); it produces no element of its own and folds into the concept whose body it fills"),
    ("ViewRenderingUsage", Classification::StructuralHelper, "the `render` view member; it lowers to the RenderingUsage kind carded as the rendering-usage concept — folds into that card"),
    ("VisibilityIndicator", Classification::LexicalHelper, "a closed keyword-set enum production; it selects a modifier/role keyword on a parent concept rather than being a written construct — folds into that concept"),
    ("XorExpression", Classification::StructuralHelper, "a precedence-cascade production of the KerML expression grammar returning the abstract Expression; the operator concept it realizes is carded by the kerml.expression.*-operator batch — the production itself is grammar precedence-climbing plumbing, not a separately-written construct"),
    ("XorExpressionReference", Classification::StructuralHelper, "an expression-grammar wrapper production that packages an owned expression as a FeatureReferenceExpression (or a trigger/self reference form); it folds under kerml.expression.feature-reference — a grammar reference-wrapper, not a written concept"),
];

/// The reviewed rationale for a reclassified name, if any.
pub fn reviewed_rationale(name: &str) -> Option<&'static str> {
    REVIEWED_RECLASSIFICATIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, r)| *r)
}

/// The auditable disposition of a raw concept in the pack (the
/// completeness contract). Every normalized source concept carries exactly one.
/// This is the load-bearing honesty field: an in-scope card-bearing concept with
/// no card is `Uncarded` (it stays in the denominator and lowers coverage)
/// instead of being silently omitted — omitting it would make the metric
/// circular.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    /// A card was minted for this concept (`normalized_concept_id` names it).
    Card,
    /// A helper rule that folds into a parent concept's grammar dependencies.
    HelperFold,
    /// Collapses into another concept (a naming alias).
    Alias,
    /// An inherited/cross-grammar duplicate that collapses into another concept.
    Duplicate,
    /// Reviewed out of scope, with rationale + reviewer.
    Exclusion,
    /// In scope but blocked from carding, with rationale (e.g. a normative
    /// locator source not yet integrated).
    Block,
    /// In scope, card-bearing, but has NO card yet: the honest low-coverage
    /// state. Lowers `card_coverage`.
    Uncarded,
}

impl Mapping {
    pub fn as_str(self) -> &'static str {
        match self {
            Mapping::Card => "card",
            Mapping::HelperFold => "helper-fold",
            Mapping::Alias => "alias",
            Mapping::Duplicate => "duplicate",
            Mapping::Exclusion => "exclusion",
            Mapping::Block => "block",
            Mapping::Uncarded => "uncarded",
        }
    }
}

/// Classify a raw grammar rule name. Reviewed overrides win; otherwise the
/// coarse heuristic: all-caps -> lexical helper; body/declaration/member/prefix
/// plumbing -> structural helper; operator/keyword rules -> operator; otherwise
/// user-facing.
pub fn classify_rule(name: &str) -> Classification {
    if let Some((_, c, _)) = REVIEWED_RECLASSIFICATIONS.iter().find(|(n, _, _)| *n == name) {
        return *c;
    }
    let is_lexical = name.chars().any(|c| c.is_ascii_uppercase())
        && !name.chars().any(|c| c.is_ascii_lowercase());
    if is_lexical {
        return Classification::LexicalHelper;
    }
    if name.ends_with("Operator") {
        return Classification::Operator;
    }
    const HELPER_SUFFIXES: &[&str] = &[
        "Body",
        "Declaration",
        "Member",
        "Prefix",
        "Part",
        "Keyword",
        "Succession",
        "Element",
    ];
    if HELPER_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return Classification::StructuralHelper;
    }
    Classification::UserFacing
}

/// Kebab-case a PascalCase rule name into a concept-ID slug: `TransitionUsage`
/// -> `transition-usage`, `PartUsage` -> `part-usage`. Digit boundaries are
/// preserved as-is.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if matches!(prev, Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit()) {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('-');
        }
        prev = Some(c);
    }
    out
}

/// Mint a concept ID `<authority>.<facet>.<slug>`.
pub fn mint_concept_id(authority: &str, facet: &str, rule_name: &str) -> String {
    format!("{authority}.{facet}.{}", slugify(rule_name))
}

/// One raw-source denominator record (matches
/// `denominator-record.schema.json`).
#[derive(Debug, Clone, Serialize)]
pub struct DenominatorRecord {
    pub source_id: String,
    pub source_kind: String,
    pub raw_name: String,
    pub normalized_concept_id: Option<String>,
    pub classification: String,
    pub classification_rationale: String,
    pub review_state: String,
    pub source_pointer: String,
    /// Auditable disposition. Every record carries one.
    pub mapping: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_target: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged_from: Vec<String>,
}

impl DenominatorRecord {
    /// Build an `xtext` denominator record for a grammar rule. `mapping` records
    /// the concept's disposition; the schema enforces `normalized_concept_id`
    /// non-null iff `mapping == Card`.
    pub fn for_xtext_rule(
        grammar: &str,
        raw_name: &str,
        classification: Classification,
        concept_id: Option<String>,
        mapping: Mapping,
        rationale: &str,
    ) -> Self {
        DenominatorRecord {
            source_id: format!("xtext:{grammar}:{raw_name}"),
            source_kind: "xtext".to_owned(),
            raw_name: raw_name.to_owned(),
            normalized_concept_id: concept_id,
            classification: classification.as_str().to_owned(),
            classification_rationale: rationale.to_owned(),
            review_state: "unreviewed".to_owned(),
            source_pointer: format!("references/sysmlv2/derived/xtext-rules.toml#{grammar}.{raw_name}"),
            mapping: mapping.as_str().to_owned(),
            mapping_target: None,
            merged_from: Vec::new(),
        }
    }

    /// Card-bearing classifications that a normalized concept can carry a card
    /// for. Mirrors [`Classification::is_card_bearing`] but named
    /// for the completeness contract.
    pub fn is_card_bearing_classification(classification: &str) -> bool {
        matches!(
            classification,
            "user-facing" | "expression" | "operator" | "semantic-only" | "library-defined"
        )
    }
}
