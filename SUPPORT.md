# Getting help

sysml-rs is a preview project maintained in the open. Support is community
best-effort — there is no commercial support offering and no response-time
guarantee.

## Where to go

**Something is broken** — open a [GitHub issue](../../issues). Please include
the version or commit, your platform, the command you ran, and a minimal
`.sysml` file that reproduces it. `sysml inspect <file>` output is usually the
fastest way to show what the tool actually saw.

**A question about using sysml-rs** — open a
[GitHub discussion](../../discussions): how to configure a project, which
command to reach for, whether a behaviour is intended, whether something is
implemented yet.

**Learning the SysML v2 language itself** — start with the companion book,
[sysmlv2-book](https://www.omg.org/spec/SysML/), a plain-English
guide to the language. For normative questions, the OMG specification at
<https://www.omg.org/sysml/sysmlv2/> is the authority; sysml-rs follows it and
does not extend it.

**A security vulnerability** — do not open a public issue. Follow
[SECURITY.md](SECURITY.md).

**Contributing a change** — see [CONTRIBUTING.md](CONTRIBUTING.md).

## Before you file

Two things resolve a large share of reports on their own:

- **Check whether the construct is implemented.** sysml-rs implements a
  substantial subset of SysML v2, not all of it. A parse error or an
  unresolved name may be a genuine gap rather than a bug in your model. Say
  what you expected either way — a gap report is still a useful report.
- **Check that setup completed.** The build needs reference material fetched
  and the tree-sitter parser generated first; see the quick start in
  [README.md](README.md). Errors that mention missing reference files or a
  missing parser are almost always an incomplete setup.
