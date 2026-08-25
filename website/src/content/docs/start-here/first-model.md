---
title: Your first model
description: Inspect, query, check, and export a shipped example model with the sysml CLI, and understand what each output is telling you.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - examples/dc-motor/DCMotor.sysml
  - examples/espresso-pump-hybrid/Physics/HydraulicConstraints.sysml
  - crates/tooling/sysml-cli
known_limitations: /sysml-rs/reference/known-limitations/
---

You have [built the tool](/sysml-rs/start-here/install/); the goal now is to
run it against a real model and understand what comes back. Every command and
output below was executed from the repository root against the shipped
examples. Outputs marked *trimmed* are shortened, never altered.

The model is `examples/dc-motor/DCMotor.sysml` — a 123-line permanent-magnet
DC motor: a `part def DCMotor` with electrical and mechanical ports, attributes
like the motor constant `Kt`, a calculation for the rotor's angular
acceleration, and two `assert constraint`s tying torque and back-EMF to the
motor constant. Open it alongside this page.

## Inspect it

`sysml inspect` shows what the tool actually saw — diagnostics, semantic
tokens, and the parse tree. `--diagnostics` limits it to the diagnostics:

```bash
./target/release/sysml --quiet inspect examples/dc-motor/DCMotor.sysml --diagnostics
```

```text
info: standard library enabled

info[CN002]: constraint 'backEmfCoupling': variable 'back_emf' has no value — result is inconclusive
  --> examples/dc-motor/DCMotor.sysml:113:8
info[CN002]: constraint 'torqueCoupling': variable 'torque' has no value — result is inconclusive
  --> examples/dc-motor/DCMotor.sysml:118:8
```

No errors: the file parses and resolves, including its imports from the
standard library. The two `info` entries are the model's assert constraints —
statically, with no values bound to `back_emf` and `torque`, their verdict is
*inconclusive*, and the tool says so rather than guessing. Each diagnostic
carries a code (`CN002`) and a source span you can click through in an editor.
This is the first command to reach for when anything behaves unexpectedly.

## Query its structure

Parsing produced a semantic graph, not just a syntax tree. `query stats`
summarises it:

```bash
./target/release/sysml --quiet query stats examples/dc-motor/DCMotor.sysml
```

```text
Elements: 233

Elements by kind:
  OwningMembership                    115
  Feature                             16
  FeatureValue                        16
  AttributeUsage                      12
  ...
  PortDefinition                       2
  AssertConstraintUsage                2
  PartDefinition                       1
  Package                              1
```

*(trimmed)* — 123 lines of text became 233 elements: every definition, usage,
expression, and membership relationship in the file, using the element kinds
of the SysML v2 metamodel.

## Check constraints to a verdict

`sysml check` evaluates constraints and reports pass/fail. Constraints need
values to evaluate, so point it at a file inside a project where the
referenced attributes are defined — here, the espresso pump example:

```bash
./target/release/sysml --quiet check examples/espresso-pump-hybrid/Physics/HydraulicConstraints.sysml
```

```text
[PASS] NonNegativeThresholds: pWarning >= 0.0 and exposureTrip > 0.0
[PASS] PositiveConductance: restrictionConductance > 0.0
[PASS] RegularizedRoot: epsRoot > 0.0

3/3 constraints passed, 0 failed
```

Note what made this work: the file names attributes like
`restrictionConductance` that are defined elsewhere in the same project, and
`check` loads the surrounding project (it has a `sysml.toml`) to resolve them.
The same file checked as a lone copy outside the project reports every
constraint `[SKIP]` — inconclusive, not failed — because the names have no
values. And `check` on `DCMotor.sysml` itself reports
`no constraints found`: its asserts sit inside a part definition that nothing
in the file instantiates, which is also why `inspect` surfaced them as *info*
diagnostics rather than verdicts.

## Export it

Everything the tool knows about the model can leave as canonical JSON:

```bash
./target/release/sysml --quiet export json examples/dc-motor/DCMotor.sysml
```

The output is one JSON document — `{"version":"1.0","elements":[...]}` with
all 233 elements. Here is the part definition, pretty-printed *(trimmed)*:

```json
{
  "id": "83927beb-2d6e-453d-bcab-0832ce127110",
  "kind": "PartDefinition",
  "name": "DCMotor",
  "owner": "21ec04ca-d6df-46b6-a026-728c1ba1dcb4",
  "spans": [{ "file": "examples/dc-motor/DCMotor.sysml", "line": 55, "col": 4 }]
}
```

Stable ids, kinds, ownership, and source spans — enough to build tooling on.
`sysml export plantuml` renders a diagram source instead; `sysml export --help`
lists the formats.

## Where next

You have seen the loop: inspect, query, check, export. From here,
[choose your path](/sysml-rs/start-here/choose-your-path/) — or go straight to
[CLI workflows](/sysml-rs/use/cli-workflows/) for the commands that execute
models rather than read them.
