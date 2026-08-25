---
title: Licensing and provenance
description: Licence terms for sysml-rs, this documentation portal, the SysML v2 Book, and the language pack.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 8d4ceb3
source_of_truth:
  - LICENSE
  - LICENSE-MIT
  - LICENSE-APACHE
---

## sysml-rs and this portal

sysml-rs — including this documentation portal's source in `website/` — is
dual-licensed under either the
[MIT License](https://github.com/RickyMillar/sysml-rs/blob/main/LICENSE-MIT) or
the
[Apache License, Version 2.0](https://github.com/RickyMillar/sysml-rs/blob/main/LICENSE-APACHE),
at your option.

## The SysML v2 Book

The [SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/) is a separate
repository and remains the canonical language-teaching source:

- Book **prose** is licensed CC-BY-4.0.
- Book **code samples and examples** are licensed MIT OR Apache-2.0.

When Book content is built into this portal it is consumed at a pinned revision
recorded in `website/content-lock.json`, and its attribution and licence
notices are preserved in the published output.

## Language pack and specification references

The machine-readable language pack published with the Book is **citation-only**:
it contains grammar-derived structure and citations into the OMG specification,
not reproduced specification prose. Its grammar intermediate representation is
derived from the SysML v2 pilot implementation's Xtext grammars (LGPL-3.0), and
the corresponding attribution notice accompanies the pack wherever it is
republished.

OMG SysML v2 and KerML specifications are the property of the Object Management
Group. This project cites them; it does not redistribute them.
