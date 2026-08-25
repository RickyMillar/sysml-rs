---
title: Imports vs dependencies
description: When to write a SysML import and when to declare a manifest dependency, and how the two work together.
scope:
  - SysML v2 / KerML
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/lang/sysml-manifest/src/dependency.rs
  - crates/tooling/sysml-resolve/src/resolver.rs
  - crates/tooling/sysml-cli/tests/inspect_workspace_integration.rs
---

You want a name defined in one place to be usable in another. Depending on where that name lives, the answer is a SysML `import`, a manifest dependency, or both — and mixing up the two is the most common source of "unresolved name" confusion.

- An **`import`** is SysML v2 language syntax. It makes names from another namespace visible inside yours, so you can write `GrindSetting` instead of `BeverageTypes::GrindSetting`. It says nothing about where the file defining `BeverageTypes` comes from.
- A **dependency** in `sysml.toml` is a sysml-rs tooling concept. It tells the toolchain which *other projects'* source files to load (and fetch, cache, and pin in `sysml.lock`). It says nothing about name visibility inside your model.

An `import` can only resolve if the defining file is loaded. Files in your own project load automatically; files in another project load only if you declare a dependency on it.

## The comparison

| | `import` (SysML v2 / KerML) | dependency (sysml-rs tooling) |
|---|---|---|
| Written in | `.sysml` source files | `sysml.toml` `[dependencies]` |
| Portable to other SysML tools | Yes — standard language syntax | No — a sysml-rs convention (Cargo-style) |
| What it controls | Name visibility between namespaces | Which projects' files the toolchain loads |
| Granularity | A namespace or a single member (`P::X` or `P::*`) | A whole project |
| Fetches anything | Never | Yes — path, git, KPAR, and registry sources |
| Versioned / pinned | No | Yes — resolved versions recorded in `sysml.lock` |
| Failure mode when missing | Unresolved-name diagnostics in the model | Resolution error from `sysml lock` / `fetch`, or unresolved names because the files never loaded |

Both layers are usually involved: the dependency loads the files, the import makes the names convenient to use.

## Example 1: an import within one project

Two files in the same project need no manifest change at all. This project was created with `sysml init --name grinder-demo` and has two source files:

```sysml
// src/types.sysml
package BeverageTypes {
    enum def GrindSetting {
        fine;
        medium;
        coarse;
    }
}
```

```sysml
// src/main.sysml
package GrinderDemo {
    import BeverageTypes::*;

    part def Grinder {
        attribute grindSize : GrindSetting;
    }
}
```

Both files are inside the project, so they load together and the import resolves:

```console
$ sysml inspect --workspace . --focus main.sysml --diagnostics
=== main.sysml (0 diagnostics) ===
  (no diagnostics)

summary: 0 diagnostics (0 errors, 0 warnings, 0 info)
```

## Example 2: the same import across a project boundary

Now move `BeverageTypes` into its own project, `beverage-types`, next to a `coffee-machine` project whose model keeps the identical `import BeverageTypes::*;`. The import syntax does not change — but the defining file no longer belongs to `coffee-machine`, so the toolchain must be told to load it. That is the dependency's job:

```console
$ cd coffee-machine
$ sysml add beverage-types --path ../beverage-types
Adding dependency 'beverage-types'
Added 'beverage-types' to .../coffee-machine/sysml.toml
```

which records in `sysml.toml`:

```toml
[dependencies.beverage-types]
path = "../beverage-types"
```

With the dependency declared, the cross-project import resolves:

```console
$ sysml inspect --workspace . --focus main.sysml --diagnostics
info: workspace dependencies: resolved 1 package(s), loaded 1 source file(s)
info: workspace: . (2 files)

=== main.sysml (0 diagnostics) ===
  (no diagnostics)

summary: 0 diagnostics (0 errors, 0 warnings, 0 info)
```

And with dependency loading switched off (`--no-workspace-deps`), the same model fails — this is exactly what a missing dependency looks like:

```console
$ sysml inspect --workspace . --focus main.sysml --diagnostics --no-workspace-deps
error[...]: name 'GrindSetting' unresolved
  = note: ensure the name is defined or imported in scope
info[IM001]: import references namespace 'BeverageTypes' (unresolved in current workspace context)
  = note: checked current file, workspace project files, and loaded standard library; check spelling/case or `[workspace].members`

summary: 2 diagnostics (1 errors, 0 warnings, 1 info)
```

## Rules of thumb

- Unresolved name for something defined **in your own project**: check the `import` (spelling, visibility, the right package name).
- Unresolved name for something defined **in another project**: check `sysml.toml` first — is the project declared under `[dependencies]`, and does `sysml lock` succeed?
- Standard-library names (`ScalarValues`, `SI`, …) need an `import` but never a dependency entry: the standard libraries ship with the tooling and are enabled per project via [`[stdlib]` in the manifest](/sysml-rs/use/sysml-toml/), not via `[dependencies]`.

For the dependency sources themselves (path, git, KPAR, registry) see [Dependencies](/sysml-rs/use/dependencies/); for pinning and caching see [Lock file and cache](/sysml-rs/use/lock-and-cache/).
