# spec-index

Generates the derived spec-reference artifacts under `references/sysmlv2/derived/`:

- **Clause-anchored spec plaintexts** (`KerML-spec-r2025-04.txt`,
  `SysML-spec-r2025-04.txt`) — generation-time inputs for citation anchoring.
- **`xtext-rules.toml`** — a rule-name → line map over the pinned Xtext grammars.
- **The language pack** (`derived/language-pack/`) — a machine-readable index of
  every SysML v2 / KerML language concept: one JSON "card" per concept with its
  grammar rules, normative clause citations, keywords, examples, and measured
  implementation-support status, plus retrieval indexes over the card corpus.

## The pack is an index, not an authority

Every card **points at** the normative sources — a spec clause locator, a
grammar rule name, a metamodel class — and paraphrases; it never replaces them.
The intended workflow, for people and for agents, is *look up, then cite the
primary source*: use the pack to find which clause governs a question, then read
and cite that clause. Where a card and the specification disagree, the
specification wins and the card is a bug.

Implementation-support claims (`support` axes on each card) are machine-derived
from test evidence, never hand-written: an axis is `validated` only when a gate
test passed for it at the current spec drop, `unknown` otherwise.

## Licensing position

The pack is **citation-only by design**: no OMG specification prose is
reproduced in any generated card, example, or index. Summaries are original
paraphrases; normative content is referenced by document + clause locator.

- The grammar IR is derived from the Xtext grammars in the
  **SysML-v2-Pilot-Implementation**, which is licensed **LGPL-3.0-or-later**
  (see `references/sysmlv2/SysML-v2-Pilot-Implementation/LICENSE` after
  fetching). The pack stores rule names, structure, and keyword literals derived
  from those grammars; this attribution notice covers that derivation.
- The metamodel/SHACL facets are derived from the OMG-published TTL
  vocabularies and shapes, referenced at pinned revisions.
- **The derived spec plaintexts (`derived/KerML-spec-r2025-04.txt` and
  `derived/SysML-spec-r2025-04.txt`) are full specification text. They are
  generation-time inputs only and must NEVER be committed to this repository or
  redistributed.** The `.gitignore` rule for `references/sysmlv2/` keeps the
  whole derived tree (plaintexts and pack alike) untracked; do not add
  exceptions for them.

## Generating

```bash
# 1. Fetch the pinned upstream sources (grammars, spec HTML, TTLs).
tools/fetch-references/fetch.sh

# 2. Derive the spec plaintexts + xtext rule map into references/sysmlv2/derived/.
cargo run -p spec-index

# 3. Generate the language pack into references/sysmlv2/derived/language-pack/.
cargo run -p spec-index -- language-pack

# 4. Inspect: pack path, freshness (clean/stale/absent), source hashes, counts.
cargo run -p spec-index -- language-pack info
```

Generation is deterministic: two clean runs at the same spec drop produce
byte-identical trees (the tree hash is in `report.json` and `language-pack
info`). Every source read is allowlisted and hash-checked against the pinned
manifest, so a silently drifted source is a hard error, not a wrong pack.

## Pack layout

```
language-pack/
  manifest.json            # spec-drop identity + pinned source hashes
  cards/<id>.json          # one card per concept, id = <authority>.<facet>.<slug>
  examples/<id>.json       # positive/negative/composed example records
  indexes/keywords.json    # BM25-ready term index over the card corpus
  indexes/dependencies.json# one-hop expansion map (grammar deps, referenced_by)
  indexes/aliases.json     # alias -> card id (rule names, keyword spellings)
  indexes/cards.jsonl      # the card corpus as one JSONL file
  indexes/denominator.jsonl# auditable disposition of every raw source concept
  retrieval/chunks.jsonl   # one retrieval chunk per card
  evals/*.jsonl            # held-out evaluation datasets (gate-validated)
  evidence.jsonl           # support evidence the shipped axes were derived from
  known-gaps.json          # reviewed implementation-limitation registry
  completeness.{json,md}   # coverage metrics, numerator AND denominator
  report.json              # counts, notes, tree hash
```

## Consumption pattern (agents)

1. **Freshness first**: `cargo run -p spec-index -- language-pack info` — use
   the pack only when `freshness` is `clean`; regenerate otherwise.
2. **Term lookup**: find candidate cards via `indexes/keywords.json` (or
   `indexes/aliases.json` for exact rule/keyword spellings).
3. **Read the card**: `cards/<id>.json` — grammar rules, clause citations,
   support axes, examples.
4. **Expand one hop** when the card's context is not enough:
   `indexes/dependencies.json` gives each card's grammar dependencies and
   `referenced_by` neighbours.
5. **Cite the primary clause**: answers about the language cite the card's
   `normative_clauses` (document + clause), not the card itself.

## Support evidence

The tracked seed `tools/spec-index/data/evidence.jsonl` is the generator's
input for the per-card `support` axes. It is machine-written by the
`language_card_examples` gate in `sysml-spec-tests` (never hand-edited), and
each record is keyed to a spec-drop *evidence epoch* (a digest of the pinned
source set), so evidence auto-invalidates when the sources change. To refresh
after an intended behaviour change:

```bash
SYSML_LP_UPDATE_EVIDENCE=1 cargo test -p sysml-spec-tests --test language_card_examples
cargo run -p spec-index -- language-pack
```

## Gates

`cargo test -p spec-index` validates generation itself (regen-diff, schema,
duplicate-ID/dangling-ref, determinism). In `sysml-spec-tests`, four gates
validate the pack against the real parser pipeline: `language_card_examples`
(every example parses/lowers/fails exactly as declared; shipped support axes
equal a live re-derivation), `language_pack_evals` (held-out answer keys stay
correct), `retrieval_eval` (deterministic BM25 recall/MRR floors), and
`language_pack_doc_links` (this documentation's card ids and paths resolve;
pack freshness). All of them **skip with a message when the sources or the
pack are absent** (fresh clone, CI without a generated pack) and run fully when
present. `SYSML_LP_PACK_DIR` points the gates (and `info`) at an alternate pack
directory.
