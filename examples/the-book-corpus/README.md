# the-book-corpus — vendored book example fixtures

Vendored, byte-identical copies of example models from the "SysML v2 book"
repository, committed here as this workspace's own regression fixtures.
Tests resolve these files via ONE unconditional in-repo path
(`<workspace>/examples/the-book-corpus/...`) — there is deliberately no
fallback to a sibling `the-book/` checkout, so the test suite is hermetic
on a fresh clone.

## Provenance

- **Source repo:** `github.com/RickyMillar/sysmlv2-book` (private)
- **Source commit:** `8545b1d15faaa2d17069e590120d4a639994b7ba`
  (the last commit touching `examples/` at vendoring time, 2026-08-22;
  the source tree was clean against it)

## File list

`coffee-machine/` — the complete coffee-machine project (17 files):

- `sysml.toml`
- `actions.sysml`, `book-views.sysml`, `brew-cycle-flow.sysml`,
  `calculations.sysml`, `connections.sysml`, `definitions.sysml`,
  `demo-analysis.sysml`, `flows.sysml`, `metadata.sysml`,
  `orchestration.sysml`, `package-structure.sysml`,
  `ports-and-interfaces.sysml`, `requirements.sysml`, `states.sysml`,
  `typing-and-specialization.sysml`, `views.sysml`

The whole project is vendored — not just the files tests name directly —
because `SysmlService::load_file` and the LSP `did_open` path both run
manifest discovery (`sysml-project::discovery::pick_mode`): opening any
file in a directory with an ancestor `sysml.toml` loads the entire
project. Dropping the manifest or any sibling `.sysml` file would change
the loaded element set and shift content-derived baseline values. The
upstream `*.sysml.layout.json` viewer-layout files are excluded: nothing
on these test paths reads them and no snapshot lists them.

`views-library/` — the eight numbered exemplars driven by
`semantic_tokens_invariants` (8 files):

- `01-minimal-view-def.sysml`, `02-view-usage-instance.sysml`,
  `03-namespace-expose.sysml`, `04-filter-and-composition.sysml`,
  `05-filter-safe-default.sysml`, `07-rendering-binding.sysml`,
  `08-viewpoint-satisfaction.sysml`, `09-eight-supertypes.sysml`

## Consumers

- `crates/testing/sysml-spec-tests/tests/identity_invariants.rs`
- `crates/testing/sysml-spec-tests/tests/cross_transport_identity_baseline.rs`
- `crates/testing/sysml-spec-tests/tests/semantic_tokens_invariants.rs`
- `crates/testing/sysml-spec-tests/tests/service_command_baseline.rs`
- `crates/testing/sysml-spec-tests/tests/perf_baseline.rs`
- `crates/tooling/sysml-service/tests/contract_get_source.rs`
- `crates/tooling/sysml-service/tests/contract_id_round_trip.rs`
- `crates/lang/sysml-runtime/tests/elaborate_metadata_invariant.rs`

## Resync procedure

These files do NOT track the book automatically. To resync:

1. Pick a settled upstream commit (clean `git status` for `examples/`);
   copy the files listed above from it, byte-identical.
2. Update the source commit recorded in this README.
3. Run the affected suites:
   `cargo test -p sysml-spec-tests --test identity_invariants
    --test cross_transport_identity_baseline
    --test semantic_tokens_invariants --test service_command_baseline`,
   `cargo test -p sysml-service --test contract_get_source
    --test contract_id_round_trip`,
   `cargo test -p sysml-runtime --test elaborate_metadata_invariant`.
4. Review EVERY snapshot diff by hand and re-bless consciously
   (`cargo insta review` — never a blind accept). Content-derived values
   (digests, element counts) changing means the upstream models actually
   changed; confirm each shift is intended before blessing.
