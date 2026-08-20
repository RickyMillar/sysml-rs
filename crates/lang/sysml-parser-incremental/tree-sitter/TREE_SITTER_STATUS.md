# Tree-sitter Grammar Status

> Single source of truth for tree-sitter grammar health. Updated by `update_status.sh`.

## Coverage

| Tier | Description | Pass | Total | % |
|------|-------------|------|-------|---|
| 1 | Standard Library (kernel+systems+domain) | 94 | 94 | 100% |
| 2 | KerML Examples | ? | ? | ? |
| 3 | SysML Examples | ? | ? | ? |
| 4 | Execution Corpus (local) | TBD | 8 | TBD |

## Generation Metrics

| Metric | Value |
|--------|-------|
| tree-sitter CLI version | 0.26.5 |
| ABI version | 14 (must use `--abi=14` for Rust crate compatibility) |
| Conflicts (declared) | 48 (G24 adds primary/index `#` conflict; removes dependency self-conflict) |
| STATE_COUNT | 15,920 |
| LARGE_STATE_COUNT | 11,321 |
| Generation time | ~16 min |
| parser.c size | 67.8 MiB |

## Internal Tests

| Corpus File | Tests | Pass |
|-------------|-------|------|
| actions.txt | 16 | 16 |
| basic.txt | 6 | 6 |
| connectors.txt | 4 | 4 |
| definitions.txt | 24 | 24 |
| expressions.txt | 21 | 21 |
| namespaces.txt | 20 | 20 |
| requirements.txt | 20 | 20 |
| spec_conformance.txt | 7 | 7 |
| state_machines.txt | 13 | 13 |
| usages.txt | 37 | 37 |
| usecases.txt | 8 | 8 |
| **Total** | **176** | **176** |

> Run `npx tree-sitter test --update` after generation to auto-fix CST expectations.

## Failing Library Files

> Run `./test_library.sh --fail-only` for current list

## Recent Changes

| Date | Change | Impact |
|------|--------|--------|
| 2026-07-15 | **G24 / B1b keyword derivation authoring** | `#keyword` on connections/ends; `::>` ReferenceSubsetting; RequirementDerivation SemanticMetadata base-type lowering; dependency endpoint lists; contextual `frame`; 176/176 corpus. |
| 2026-02-25 | **100% library coverage** | Added `"accept"` to `_keyword_name`; mandatory terminator for `connector_usage` fixes early reduce. 94/94 library (100%). -324 states, -1 conflict. |
| 2026-02-24 | **Lambda unification** | Replaced `arrow_lambda_expression` + `lambda_parameter_list` with `function_body` reuse. collect/select expressions use function_body. -443 states from 13,116 peak. 92/94 library (97%). |
| 2026-02-24 | **Binding rule fixes** | Added own-multiplicity to binding_usage, `bind` keyword pattern, `expose` visibility. Fixed 11 library files. |
| 2026-02-24 | **Expression precedence restructure** | Inlined invocation args into arrow_expression RHS to fix shift-reduce conflict. invocation_expression at prec(ARROW+1). |
| 2026-02-20 | **Phase 4: Control-flow node merge** | Merged 5 nodes (merge/decide/fork/join/perform) into `control_flow_node` with keyword field. -202 states. |
| 2026-02-20 | **Phase 4: Conflict pruning** | Pruned conflicts from ~70 to 60. Experiment showed most are unnecessary but some required after rule merges. |
| 2026-02-19 | **Phase 3: Rule merging optimization** | Merged 5 standard usages into `standard_usage` with keyword field |
| 2026-02-19 | **Phase 2: Hidden subrule extraction** | `_usage_name` only (header caused >65k states) |
| 2026-02-19 | `_usage_header`/`_def_header` abandoned | usage_prefix keyword overlap + GLR = state explosion |
| 2026-02-19 | Created `_usage_name` hidden subrule | Shared name portion across ~20 rules |
| 2026-02-19 | Created `tree-sitter.json` | ABI 15 support |
| 2026-02-19 | Fixed `alias_decl` in `_root_member` | Aliases work at file root level |
| 2026-02-19 | Fixed `objective_requirement` trailing `;` | `;` now optional after body |
| 2026-02-18 | Upgraded tree-sitter CLI 0.22.6 → 0.26.5 | `--no-bindings` removed, `--update` flag added |
| 2026-02-18 | Created execution corpus (8 files) | Tier 4 baseline: 7/8 (87%) |
| 2026-02-18 | Expanded internal corpus (6 → 126 tests) | 126/126 pass |
| 2026-02-18 | Fixed corpus CST expectations | Used `tree-sitter test --update` |
| 2026-02-18 | Fixed highlights.scm query | Removed stale `path:` field reference |

## Architecture: Hidden Subrules (Performance-Critical)

### Why Hidden Subrules Matter

Tree-sitter uses LR parsing. Each `optional()` in a `seq()` doubles the number of parse paths.
With N optionals across M rules, the state table grows as **M × 2^N**. For this grammar:
- 30+ rules × 2^8 optionals = massive state explosion
- parser.c was 61 MB, 13,572 states, generation took 30+ minutes

### Current Optimizations

**Rule Merging (Phase 3):** 5 structurally identical standard usages (part, attribute, item,
occurrence, ref) merged into `standard_usage` with a `keyword` field. This eliminates 4 duplicate
rule state sets without hidden subrule GLR inflation. Consumers distinguish via keyword field value.
See `~/.claude/skills/learned/treesitter-rule-merging-optimization.md` for the full pattern.

**Control-Flow Node Merge (Phase 4):** 5 control-flow nodes (merge_node, decision_node, fork_node,
join_node, perform_action) merged into `control_flow_node` with keyword field. Saves 202 states
(12,212 → 12,010). Same pattern as standard_usage merge.

**Conflict Pruning (Phase 4):** Experiment proved most conflicts unnecessary with unmodified grammar,
but some ARE required after rule merges. Pruned from ~70 to 60 declarations. Further pruning possible
with systematic one-by-one elimination (~18 min per iteration).

### Current Hidden Subrules

None. All extraction attempts were abandoned due to state count explosion.

### Abandoned Hidden Subrules (DO NOT RE-ATTEMPT)

```
_usage_name (short_name + name) — ABANDONED
  Reason: Needs [$._usage_name] GLR conflict for short_name • _name lookahead.
  That conflict × ~20 calling contexts inflated states from ~13k to 78,122 (limit 65,535).

_usage_header (vis + abstract + prefix) — ABANDONED
  Reason: usage_prefix contains "ref", "in", "out", "inout" which overlap with
  usage rule keywords. GLR conflict inflation: 74,476 states.

_def_header (vis + abstract) — ABANDONED
  Reason: "abstract" in both def and usage headers with different continuations.
  Cross-conflict + GLR inflation.
```

### CRITICAL: Rules for Keeping Generation Under State Limit

**DO:**
- Keep ALL optionals (vis, abstract, prefix, short_name, name) INLINE in each rule
- Use `defRule` helper in patterns.js for definition rules
- Add conflict declarations for new rules with `repeat($._feature_specialization)`
- Use `--report-states-for-rule -` to identify expensive rules after generation

**DON'T:**
- Create hidden subrules that need GLR conflict declarations — inflates states × calling contexts
- Create hidden rules that can match the empty string — tree-sitter rejects them
- Add new GLR conflict declarations on shared subrules

### Empty-String Rule Constraint

Tree-sitter **forbids** rules matching the empty string. If all children are optional,
use `choice()` to require at least one token, then wrap in `optional()` at call sites:

```javascript
// WRONG: all children optional → matches empty → ERROR
_my_rule: ($) => seq(optional(a), optional(b), optional(c))

// CORRECT: choice ensures at least one token
_my_rule: ($) => choice(
  seq(a, optional(b), optional(c)),
  seq(b, optional(c)),
  c,
)
// Call site: optional($._my_rule)
```

## Remaining Failures (0/94)

None! 100% library coverage achieved.

### How the last 2 failures were fixed (2026-02-25)

**StatePerformances.kerml** (1 error) and **TransitionPerformances.kerml** (3 errors):

1. **`accept` as feature name**: Added `"accept"` to `_keyword_name` (alongside `"entry"`, `"exit"`, `"do"`). This lets `feature_chain` match `accept` as a name in succession sources, step names, etc.

2. **connector_usage early reduce**: `connector [0..1] transitionLink to [1..*] trigger;` was parsed as `connector [0..1]` (complete) + separate members. Root cause: `optional(";")` allowed the parser to reduce connector_usage without consuming any tokens after the multiplicity. Fix: replaced `optional($.usage_body), optional(";")` with mandatory `choice(seq($.usage_body, optional(";")), ";")` — every connector must end with body or semicolon (per SysML TypeBody rule). This eliminated the shift-reduce conflict entirely (connector_usage self-conflict no longer needed).

## Lessons Learned

### Mandatory terminators prevent early reduce in optional chains
When a rule has a long chain of optional fields, the LR parser can reduce (exit) at any point rather than continuing to shift more tokens. This causes patterns like `connector [0..1] transitionLink to [1..*] trigger;` to parse incorrectly — the parser exits after `connector [0..1]`. Fix: replace `optional(body), optional(";")` with `choice(seq(body, optional(";")), ";")` to require a terminator. The parser can't reduce until it finds `;` or `{`, forcing it to consume intervening tokens. This also eliminates the need for GLR self-conflict declarations.

### Lambda bodies are just function bodies (xtext spec insight)
The xtext spec defines lambda bodies as `CalculationBody` — the same as function/calc bodies. `in x;` inside a lambda is just a `DefaultReferenceUsage` with direction=in, parsed as a regular body member. This means `function_body` handles ALL lambda patterns uniformly: `->minimize { doc ... in x; eval(x) }`, `->forAll { in x : Type; x > 100 }`, `->selectOne { in ref a { doc ... } expr }`, `->collect { expr }`. Creating custom lambda rules (`arrow_lambda_expression`, `lambda_parameter_list`) is unnecessary and increases state count.

### Inlining optional arguments eliminates shift-reduce conflicts
When an arrow expression's RHS can be either `name` or `name(args)`, putting these as separate `choice()` branches creates an LR conflict: after reducing the name, the parser can't look ahead to see `(`. Inlining `optional(seq("(", args, ")"))` after the name eliminates this structural ambiguity completely.

### `tree-sitter test --update` auto-fixes corpus expectations
Run `npx tree-sitter test --update` after grammar changes. It rewrites expected CST in corpus files to match actual parser output. Only tests with ERROR/MISSING nodes need manual fixes.

### Query files break on grammar changes
`highlights.scm` and other `.scm` files reference field names and node types. When grammar removes/renames these, tests fail with "Invalid field name" or "Impossible pattern" errors. Always check after grammar updates.

### Key CST patterns
- `type_ref` wraps single identifiers in `feature_chain`: `(type_ref (feature_chain (identifier)))`
- `import_decl` uses bare `(identifier)` children, no field labels
- `multiplicity` may nest inside `typing` rather than being a sibling

## Optimization Workflow

### Step 1: Identify Expensive Rules
After a successful generation, use `--report-states-for-rule -` to find the most expensive rules:
```bash
npx tree-sitter generate --report-states-for-rule - 2>&1 | sort -t: -k2 -n -r | head -20
```
Focus on the top offenders. Optimizing 1-2 rules can cascade improvements across the grammar.

### Step 2: Refactor Iteratively
The [wiki](https://github.com/tree-sitter/tree-sitter/wiki/Tips-and-Tricks-for-a-grammar-author) emphasizes:
"Refactor, regenerate, check state counts, rinse and repeat" — some refactors INCREASE states.

### Step 3: Techniques (in order of effectiveness)
1. **Extract hidden subrules** from shared optional clusters (Issue #656 pattern)
   - Only works when subrule FIRST sets don't overlap with calling-context keywords
   - Use `choice()` to prevent empty-matching (see above)
2. **Use `--report-states-for-rule -`** to find and target the most expensive rules
3. **Ensure keyword extraction** is active (`word: ($) => $.identifier`)
4. **Define comments as named rules** rather than inline regex in extras
5. **Simplify grammar over spec purity** — merge overlapping rules where possible
6. **Prune stale conflicts** — comment out all, regenerate, add back only required ones

### Step 4: Measure
Always compare before/after:
- State count: check generation output
- Generation time: `time npx tree-sitter generate`
- parser.c size: `ls -lh src/parser.c`

### References
- [Tips and Tricks wiki](https://github.com/tree-sitter/tree-sitter/wiki/Tips-and-Tricks-for-a-grammar-author)
- [Issue #656 (COBOL)](https://github.com/tree-sitter/tree-sitter/issues/656) — 15+ min → 17s
- [Issue #693 (Ada)](https://github.com/tree-sitter/tree-sitter/issues/693) — 90+ min, grammar simplification
- [tree-sitter-haskell 50x](https://owen.cafe/posts/tree-sitter-haskell-perf/) — runtime perf via C rewrite

## Known Issues

- Single-element `choice()` warnings in expressions (cosmetic, from binary expr factories)
- Conflict count needs further pruning
- Named `assume constraint <name>` has grammar ambiguity — name parsed as separate `feature_declaration`
- Internal test CST expectations auto-fixed with `tree-sitter test --update`
