# Tree-sitter Grammar Optimization Guide

This document explains why the SysML v2 tree-sitter grammar has the shape it does,
what optimization techniques work (and don't), and how to approach future changes.

## Grammar Architecture

The grammar generates a ~34 MB `parser.c` (3.4 MB WASM) with ~11,950 LR parse states.
This is large because SysML v2 has:
- 40+ usage/definition rule types with similar but distinct syntax
- Unordered feature specializations (`repeat(choice(14 alternatives))`)
- Rich expression tower (15 precedence levels)
- Keyword-heavy syntax where many tokens serve dual purposes

## Optimization Techniques

### What Works

#### 1. Mandatory Terminator Pattern (Proven, -635 states)

**Before:**
```js
optional($.usage_body),
optional(";"),
```

**After:**
```js
choice(
  seq($.usage_body, optional(";")),
  ";",
),
```

**Why it works:** The LR parser can't tell when `optional(body), optional(";")` ends --
the rule could reduce after just the keyword/name with no body or semicolon. The
mandatory terminator forces the parser to find either `{` (body) or `;` before
reducing, eliminating premature-reduce ambiguity.

**Safety rule:** Only apply to rules with **unambiguous keywords** that don't double
as usage prefixes. See the safety table below.

| Safe to Apply | Unsafe -- Do NOT Apply |
|---------------|----------------------|
| succession_decl ("succession") | standard_usage ("part", "attribute", etc.) |
| binding_usage ("binding") | port_usage ("in", "out", "inout") |
| defRule() factory (keyword + "def") | state_usage ("state") |
| kerml_definition (KerML keywords) | feature_declaration (no keyword) |
| kerml_usage ("step", "expr", etc.) | feature_redefinition (no keyword) |
| assert_constraint_usage ("assert") | usage (generic fallback) |
| definition generic ("calc def", etc.) | flow_connection_usage ("flow") |
| case_usage ("case"/"use case") | objective_requirement ("objective") |
| actor_usage ("actor") | satisfy_requirement ("satisfy"/"verify") |
| case_def ("case def"/"use case def") | interface/requirement/constraint_usage |

**Root cause of unsafety:** These rules allow bare keyword matching where the keyword
also appears as a `usage_prefix` token or where the rule reduces after just the
keyword. Example: `in port dataIn : DataPort;` -- `in` matches port_usage keyword,
but with mandatory terminator it must see `{` or `;` immediately, which fails.

#### 2. Rule Merging WITHOUT field() (Proven, -200+ states per merge)

Merge structurally identical rules that differ only by keyword into one rule with a
bare `choice()` keyword:
- `standard_usage`: merged 5 rules (part, attribute, item, occurrence, ref) -> 1
- `standard_def`: merged 9 rules (part, attribute, port, connection, interface, item, allocation, occurrence, flow) -> 1
- `flow_connection_usage`: merged flow + succession flow -> 1
- `control_flow_node`: merged 5 nodes (fork, join, merge, decision, done) -> 1
- `binding_usage`: merged "of"/"bind" branches -> `choice("of", "bind")`

**CRITICAL: Do NOT use `field("keyword", choice(...))` on merged keywords.**
The `field()` annotation forces the LR engine to create distinct parse states for
each alternative to track which field value was matched. This INCREASES states:
- flow-merge with `field()`: +540 states (WORSE)
- flow-merge without `field()`: -336 states (WINNER)
- def-merge with `field()`: -552 states
- def-merge without `field()`: -580 states (28 more saved)

Use bare `choice()` and let consumers inspect the keyword token directly.

#### 3. SMALL_STATE_THRESHOLD Patch (Proven, -19 MB parser.c, -57% WASM)

Tree-sitter's generator hardcodes `SMALL_STATE_THRESHOLD = 64` in `render.rs`.
States with >64 non-empty entries use a dense `[STATE][SYMBOL]` 2D array; smaller
states use a compact sparse format. Our grammar has 407 symbols, so states with
65-200 entries waste ~60% of their dense array on empty slots.

**Patching threshold to 200:** LARGE_STATE_COUNT drops from 8,769 to 32. Only ~32
states actually have >200 entries. The rest move to sparse format.

| Metric | Before (threshold=64) | After (threshold=200) |
|--------|----------------------|----------------------|
| parser.c | 54.1 MB | 34.9 MB (-35.5%) |
| WASM | 7.9 MB | ~3.6 MB (-55%) |
| LARGE_STATE_COUNT | 8,769 | 32 |
| Parse behavior | identical | identical |

**Runtime impact:** Sparse lookup is O(groups) linear scan (~15-33 iterations) vs O(1)
array index. Already used for 4,159 existing small states. Negligible for incremental
IDE parsing.

**How to apply:** Build a patched tree-sitter CLI with modified `render.rs` in the
`tree-sitter-generate` crate. See `/tmp/ts-cli-patch/` for the build setup.

#### 4. Hidden Subrule Extraction for Connection Portions (Proven, -671 states)

Previous hidden subrule extractions failed catastrophically on HEADER portions
(65k+ state explosion). However, extracting CONNECTION portions anchored by unique
keywords works:

```js
// Extract the post-"then" connection as a hidden rule
_succession_connection: ($) => seq(
  optional(seq(optional(choice("from", "first")), optional($.multiplicity),
    field("source", choice($.feature_chain, $.qualified_name)))),
  "then",
  optional($.multiplicity),
  field("target", choice($.feature_chain, $.qualified_name)),
),
```

**Why this works but headers don't:** The "then" keyword is unique to succession_decl
and unambiguously anchors the extraction point. Header keywords ("abstract", visibility,
usage_prefix) overlap across 15+ rules, causing exponential state splitting.

**Required:** Add `[$._succession_connection]` conflict to resolve multiplicity
ambiguity between the outer succession_decl and the inner connection.

#### 5. Conflict Removal After Structural Changes (Cosmetic)

After applying mandatory terminators or rule merges, the generator reports newly
unnecessary self-conflicts. Removing them produces byte-identical `parser.c` -- it's
purely code hygiene, not a performance optimization. Currently at 35 conflicts (down
from 60+).

### What Does NOT Work

#### 1. Expression Rule Merging (0 states saved)

Merging single-operator expression rules (e.g., folding `null_coalesce_expression`
into `or_expression`) saves zero states. Named rules don't have per-rule state
overhead -- the states come from the `$._expression op $._expression` structure,
not the rule name.

#### 2. Expression Precedence Ladders (WORSE -- state explosion)

Replacing `$._expression` operands with next-tighter-level types (PEG-style ladder)
**increases** states. The hidden `_*_or_below` choice rules add intermediate states.
Precedence ladders are a PEG/recursive-descent optimization; tree-sitter's LR
generator already handles precedence optimally via `prec.left()/prec.right()`.

#### 3. Ordered Feature Specializations (Breaks language, +20 states)

Replacing `repeat($._feature_specialization)` with a structured sequence that
enforces typing->multiplicity->supertypes->redefinition order **breaks 141 library
files**. The SysML specification allows feature specializations in any order
(e.g., `[1..*] : Type` puts multiplicity before typing). The `repeat(choice(14))`
pattern and its self-conflicts are fundamentally necessary.

#### 4. Hidden Subrules (_usage_header, etc.) (>65k state explosion)

Extracting common usage/definition header patterns into hidden `_` rules causes
catastrophic state explosion due to keyword overlap x GLR conflict inflation
across 15+ calling contexts:
- `_usage_name`: 78k states
- `_usage_header`: 74k states
- `_def_header`: cross-conflict on "abstract"

#### 5. `alias()` (Increases states)

Using tree-sitter's `alias()` to rename nodes adds contexts to the state machine,
increasing state count.

#### 6. `field()` on keyword choices (Increases states)

Wrapping `choice("kw1", "kw2")` in `field("keyword", ...)` forces the LR engine to
track which alternative matched, creating distinct parse states per alternative.
This consistently increases state count. See Rule Merging section above.

#### 7. `prec()` on Disambiguated Tokens (No-op)

Adding precedence to tokens that are already unambiguous has no effect.

#### 8. Universal Conflict Pruning (0 states)

Removing conflict declarations that the generator calls "unnecessary" produces
byte-identical parser.c. Conflicts only affect whether generation succeeds or
fails -- they are not embedded in the generated parser.

## State Budget

Top state consumers (from `--report-states-for-rule -` diagnostic, post-Round 2):

| Rule | States | % | Notes |
|------|--------|---|-------|
| succession_decl | 1,858 | 15% | Reduced by mandatory terminator |
| binding_usage | 1,036 | 8.4% | Reduced by merge + terminator |
| alias_decl_repeat1 | 779 | 6.3% | From repeat(_feature_specialization) |
| connector_usage | 760 | 6.2% | Already has mandatory terminator |
| flow_connection_usage | 460 | 3.7% | Merged with succession_flow (Round 3) |
| standard_usage | 462 | 3.7% | Merged from 5 rules |
| control_flow_node | 426 | 3.4% | Merged from 5 rules |
| usage (generic) | 418 | 3.4% | |
| feature_declaration | 363 | 2.9% | |
| 12 expression rules | 281 each | 27.3% | Optimal -- cannot reduce further |

## Self-Conflicts: Why They're Necessary

Usage/definition rules with `repeat($._feature_specialization)` need self-conflict
declarations in `helpers/conflicts.js`. After parsing a keyword/name, the parser sees
a token like `[` and can't tell if it starts another specialization (shift into the
repeat) or something after the repeat (reduce). The GLR conflict declaration tells
tree-sitter to explore both paths.

**These cannot be eliminated** without restructuring to ordered specializations, which
breaks the language. They are the irreducible cost of SysML's unordered syntax.

Note: After mandatory terminator and rule merge optimizations, many former
self-conflicts become unnecessary. Always run generation and check for "unnecessary
conflict" warnings after structural changes.

## Experiment History

### Round 1 (Feb 27, 2026) -- 5 experiments
| Experiment | Result | Notes |
|-----------|--------|-------|
| extras-collapse | -793 KB only | Marginal |
| conflict-prune | FAIL | Generation abort with empty array |
| connector-dedup | +351 states | WORSE |
| infinity-prec | No effect | prec() on disambiguated token |
| state-report | Diagnostic | Produced per-rule state counts |

### Round 2 (Feb 27-28, 2026) -- 9 experiments
| # | Experiment | States | Size | Verdict |
|---|-----------|--------|------|---------|
| 9 | selective-terminator | 12,353 (-635) | 49.5 MB (-9.7 MB) | **WINNER -- committed** |
| 2 | binding-simplify | 12,693 (-295) | 53.2 MB (-6 MB) | Winner (subset of 9) |
| 1 | succession-terminator | 12,782 (-206) | 53 MB (-6 MB) | Winner (subset of 9) |
| 6 | typing-mult-split | 12,935 (-53) | 56 MB (-3 MB) | Small gain, regresses when stacked |
| 3 | universal-terminator | 11,969 (-1,019) | 41 MB (-18 MB) | Too aggressive (50% lib fail) |
| 4 | expression-collapse | 12,988 (0) | 59.1 MB | No benefit |
| 5 | conflict-prune | 12,988 (0) | 59.2 MB | Cosmetic only |
| 7 | feature-spec-structured | 13,008 (+20) | 58.4 MB | FAIL (141 files broken) |
| 8 | expression-ladder | N/A (killed) | N/A | FAIL (PEG technique) |

### Round 3 (Feb 28, 2026) -- 6 experiments
| # | Experiment | States | Size | Verdict |
|---|-----------|--------|------|---------|
| 6 | def-merge-nofield | 11,773 (-580) | 46 MB (-3.5 MB) | **WINNER** |
| 5 | flow-merge-nofield | 12,017 (-336) | 46 MB (-3.5 MB) | **WINNER** |
| 1 | conflict-cleanup | 12,353 (0) | 49.5 MB | Cosmetic (62->35 conflicts) |
| 4 | state-report-v2 | 12,353 (0) | 49.5 MB | Diagnostic |
| 2 | flow-merge (field) | 12,893 (+540) | 52.6 MB (+3.1 MB) | WORSE (field() penalty) |
| 3 | def-merge (field) | 11,801 (-552) | 46 MB (-3.5 MB) | Winner (nofield is better) |
| **Combined** | **11,437 (-916)** | **44 MB (-5.5 MB)** | **Perfectly additive, committed** |

### Round 4 (Mar 2, 2026) -- 7 experiments (2 tracks)

**Track A: SMALL_STATE_THRESHOLD patch (no grammar changes)**
| # | Experiment | States | Size | WASM | Verdict |
|---|-----------|--------|------|------|---------|
| A | threshold=200 | 12,928 (0) | 34.9 MB (-19.2 MB) | ~3.6 MB | **WINNER -- huge size reduction** |

**Track B: Grammar optimizations on new (unoptimized) rules**
| # | Experiment | States | Size | Verdict |
|---|-----------|--------|------|---------|
| B1+B2 | case_usage + use_case_usage merge + terminator | 12,674 (-254) | 49.6 MB | **WINNER** |
| B3 | actor_usage mandatory terminator | 12,928 (0) | 54.1 MB | No effect (ambiguity already resolved) |
| B4 | case_def + use_case_def merge | 12,878 (-50) | 51.3 MB | **WINNER** (small) |
| B6 | succession_decl connection extraction | 12,257 (-671) | 50.4 MB | **WINNER -- biggest grammar gain** |
| **C1** | **All B combined** | **11,949 (-979)** | **50.5 MB** | **Perfectly additive, 163/163 pass** |
| **C2** | **A + all B combined (FINAL)** | **11,949 (-979)** | **34.2 MB** | **163/163 pass, WASM 3.4 MB** |

**Key findings:**
- B6 (succession extraction) is the first successful hidden subrule extraction -- previous
  attempts on headers caused 65k+ state explosion. Connection portions with unique keywords work.
- B3 (actor_usage terminator) saved 0 states -- the self-conflict was already resolving the
  ambiguity. Removing the conflict declaration alone was a no-op.
- Track A (threshold) is the single biggest win: -19.2 MB from a 1-line change to tree-sitter.
- Track B grammar opts alone: -979 states, -3.6 MB. Combined with Track A: -19.9 MB total.
- **UX impact:** `use_case_def` and `use_case_usage` node types removed. Consumers distinguish
  by scanning for "use" keyword token in `case_def`/`case_usage` (same pattern as other merges).

### Cumulative Optimization (from original 12,988 states / 59.2 MB)
| Optimization | States Saved | Cumulative |
|-------------|-------------|------------|
| selective-terminator (R2) | -635 | 12,353 |
| def-merge-nofield (R3) | -580 | 11,773 |
| flow-merge-nofield (R3) | -336 | 11,437 |
| conflict-cleanup (R3) | 0 (cosmetic) | 11,437 |
| 7 new rules added post-R3 | +1,491 | 12,928 |
| case_usage merge + terminator (R4 B1+B2) | -254 | 12,674 |
| case_def merge (R4 B4) | -50 | 12,624 |
| succession extraction (R4 B6) | -671 | 11,949 |
| SMALL_STATE_THRESHOLD=200 (R4 A) | 0 states, -19 MB | 11,949 |
| **Total** | **-1,039 states** | **11,949 states, 34.2 MB (-42.2%), WASM 3.4 MB (-57%)** |

## Future Optimization Targets

1. **Upstream SMALL_STATE_THRESHOLD PR** -- our patch to tree-sitter-generate could
   benefit all large grammars. Consider submitting a PR that makes the threshold
   configurable or auto-calculated from SYMBOL_COUNT.

2. **More connection-portion extractions** -- the B6 technique (hidden subrules for
   connection portions with unique keywords) could apply to other connector-like rules.
   Look for rules with unique anchor keywords separating header from connection.

3. **requirement_usage + constraint_usage merge** -- structurally similar, different
   bodies. Needs testing since body types differ.

4. **Typing-multiplicity split** -- -53 states independently but causes -61 library file
   regression when stacked. **Do not apply** without larger restructuring.

5. **succession_decl** -- still large despite B6 extraction. The header portion
   (typing/multiplicity/modifiers/supertypes/redefinition) accounts for most states.

## References

- `codex-ts-review.md` -- Independent performance review with additional ideas
- `experiments/state-report-2026-02-27.txt` -- Per-rule state diagnostic (pre-optimization)
- `TREE_SITTER_STATUS.md` -- Grammar status and coverage tracking
- Tree-sitter issues #656 (optional explosion), #693 (Ada timeout)
- Tree-sitter wiki "Tips and Tricks" page
