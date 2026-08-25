---
title: Known limitations
description: Verified, versioned defects and gaps in the current sysml-rs build, with their observed behaviour and workarounds.
scope:
  - sysml-rs implementation
  - Experimental / partial support
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/lang/tree-sitter-sysml
  - crates/lang/sysml-core
  - crates/lang/sysml-runtime
  - crates/tooling/sysml-cli
---

Every entry on this page was re-verified against a release build of the commit
named above: the behaviour was reproduced on that build before being listed.
Entries are removed when a fix lands, not when one is planned. Broader
positioning ("what is partial by design") lives in
[What sysml-rs is](/sysml-rs/start-here/what-is-sysml-rs/); this page is for
specific, observable defects a user may hit.

## Language and parser

### `expose` is accepted inside a `view def` body

Per the OMG grammar, `expose` belongs only in view *usage* bodies; a view
*definition* body takes filters and `render` members. sysml-rs currently
parses `expose` inside `view def` with no diagnostic, so a spec-invalid model
checks clean. Keep `expose` in view usages; a grammar fix is queued for the
next parser regeneration batch.

### `satisfy` of a viewpoint is rejected

A viewpoint is a kind of requirement, so `satisfy someViewpoint by view;` is
legal SysML v2. sysml-rs reports `VC010: unknown requirement` for it.
Workaround: none within `satisfy`; document the relationship in prose or a
comment until fixed.

### `parallel` state bodies are rejected

The `parallel` keyword in a state definition body (orthogonal regions) fails
to parse: `Unexpected keyword 'parallel' in state definition`. Model
orthogonal behaviour as separate concurrent state machines for now.

### Import bodies and filtered imports are rejected

- `import` with a body (e.g. relationship annotations on the import) fails to
  parse, and the error blames the visibility keyword rather than the body.
- Filtered namespace imports such as `import P::**[@SomeMetadata];` fail with
  a syntax error at `@`.

Both forms are standard SysML v2. Workaround: unfiltered imports plus explicit
element filters where the consuming construct supports them.

### Textual representations are not implemented

`rep ... language "..."` (KerML textual representation, e.g. embedding OCL or
other languages) fails to parse.

### A synthesized control-node name can leak into resolution

Action models with implicit control nodes can produce an `AX006` diagnostic
referring to an internal `$ctrl_…` name the user never wrote. If you see a
diagnostic about a `$`-prefixed name, it is an implementation artefact, not
your model.

### `for x in xs` loops always report AX009

A well-formed `for` loop over a collection fires
`AX009: no collection reference` even when the collection reference is
present and correct. The diagnostic is noise in this case; the loop itself is
parsed.

## Projects and packaging

### Relative paths with a directory component fail project discovery

From a project root, `sysml check src/main.sysml` fails with a discovery
error (`discovery root is not a directory`, exit 2). An absolute path works,
and so does the bare filename run from inside `src/`. Use absolute paths in
scripts until this is fixed.

### A default-stdlib `sysml package` archive cannot be consumed

An archive built from a project using the default standard libraries records
the ten standard-library URLs as usages, and consuming it as a `kpar`
dependency fails with a dependency cycle through the standard library.
Workaround (verified): the producing project sets `[stdlib] exclude = ["all"]`
before packaging; consumption then works, with the archive checksum pinned in
the consumer's lock file. See [KPAR packaging](/sysml-rs/use/kpar/).

### `sysml package` archives are not byte-reproducible

The archive metadata embeds a creation timestamp, so two builds of identical
content have different checksums. Expect `kpar` checksums in lock files to
churn when a dependency is repackaged, even without content changes.

## Runtime and CLI

### `simulate` can misreport the initial state and transition targets

The simulation trace can print an initial state that differs from the model's
declared initial transition, and some transition lines name the wrong target
state even when the same run's available-transitions output names the right
one. Treat state names in `simulate` traces with suspicion until this is
fixed; verification verdicts are computed separately and are not known to be
affected.

### `sysml verify` prints `[ERR ]` and exits 3 on a passing verdict

A verification run whose verdict is *pass* still prints an `error:`-prefixed
line containing `verdict: pass` and exits with code 3. Scripts must parse the
verdict text rather than the exit code for now.

### `sysml analysis --set` is silently ignored

Overriding an attribute with `--set name=value` produces output byte-identical
to a run without the flag; the override does not reach the model. Edit the
model (or use a redefinition) to change analysis inputs until this works.

## Not defects, but easy to trip on

- **No binary release yet.** Install is
  [from source](/sysml-rs/start-here/install/) until a release is cut.
- **Language coverage is partial.** Unsupported syntax surfaces as a parse
  diagnostic; it is not silently accepted (with the `expose` exception above).
- **The desktop workbench and diagram surfaces are previews.** The CLI and
  LSP are the stable surfaces.

## Reporting

Found something not listed here? Please
[open an issue](https://github.com/RickyMillar/sysml-rs/issues) with the model
snippet and the command you ran.
