# fetch-references

Reconstructs `references/sysmlv2/` and `libraries/standard/` from pinned
upstream sources.

The public repository does not vendor the OMG specification materials, the OMG
pilot implementation, or the standard model library derived from it.
`manifest.toml` records where each piece comes from (a git repository at an
exact commit, or a published OMG URL) and what the result must hash to;
`fetch.sh` rebuilds it and verifies it.

```sh
tools/fetch-references/fetch.sh           # reconstruct (default mode)
tools/fetch-references/fetch.sh fetch     # the same, named explicitly
tools/fetch-references/fetch.sh verify    # check an existing tree (no network)
tools/fetch-references/fetch.sh list      # print the pinned inventory
```

A fresh clone must run `fetch` before it can compile at all. `sysml-core`'s
build script panics without the two vocabulary TTLs, the two shapes TTLs, the
two XMI files, or the API-Services metamodel schema directory. (The Xtext
grammars are *not* what makes the build fail — they are guarded by an
`exists()` check, and their absence only downgrades cross-reference validation
to a `cargo:warning`. Fetch `pilot-implementation` anyway so that validation
actually runs.)

The same run reconstructs `libraries/standard/`, which the runtime loads
whenever `SYSML_LIBRARY_PATH` is unset — without it, name resolution against
standard types has nothing to resolve against.

Two items are not verbatim copies of upstream and say so with an
`upstream_sha256` beside a `patch`: `example-models` and `standard-library`.
That pair of hashes is the disclosure that third-party material was modified.
Never close a gap between them by re-pinning `sha256`.

Self-test: `tools/fetch-references/selftest.sh` (offline, seconds).
