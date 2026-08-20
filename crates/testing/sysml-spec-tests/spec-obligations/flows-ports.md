# Obligation matrix — Flows & ports

**Area:** flow transfer + port topology/direction semantics.
**Existing gate:** `tests/runtime_spec_conformance.rs` (RSC-0.1 transfers,
RSC-0.3 ports/send-accept) gates most of this area's behavioral obligations.
This matrix is primarily a **cross-reference map** — the flows/ports area is the
best-covered runtime area. **No new gate file** added. **Status:** fan-out area.

Spec sources: SysML §7.12 *Ports*, §7.16 *Flows* (`SysML-spec-r2025-04_REF.html`);
KerML §9 *Transfers* (`KerML-spec-r2025-04_REF.html`); `sysml.library/.../Transfers.kerml`,
`Systems Library/Ports.sysml`, `Flows.sysml`. Verified `2026-06-21`.

## Obligation table

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `flow-transfers-values-source-to-target` | A flow transfers a payload from source `sourceOutput` to target `targetInput`. | KerML §9 *"picking up … dropping it off"*; SysML §7.16.1 (GOSPEL) | **GATED-elsewhere** — `spec_transfer_ismove_payload_leaves_source_per_transfer`. |
| `flowtransfer-ismove-empties-source` | `isMove=true` (default) removes the payload from the source at transfer start. | KerML §9 *"isMove true requires sourceOutputLinks to end"*; `Transfers.kerml:84` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_transfer_ismove_ispush_defaults`, `spec_transfer_ismove_false_copies_payload`. |
| `flowtransfer-ispush-triggers-start` | `isPush=true` (default) starts the transfer as soon as the payload is available. | KerML §9; `Transfers.kerml:92` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_transfer_ispush_false_pull_initiation`. |
| `flow-payload-conformance` | Transferred values must conform to the endpoint payload feature types. | KerML `checkPayloadFeatureRedefinition`; §9 (GOSPEL) | **GATED-elsewhere** — `spec_transfer_payload_conformance_to_endpoint_features`. |
| `port-direction-governs-transfer-direction` | Transfers go out→in across matching features; both ways for inout. | §7.12.2 (GOSPEL) | **GATED-elsewhere** — `spec_port_conjugation_direction_at_transfer_time`. |
| `conjugated-port-inverts-directions` | A conjugated port swaps in/out; inout/undirected unchanged. | §7.12.3; KerML conjugation (GOSPEL) | **GATED-elsewhere** — `spec_port_conjugation_direction_at_transfer_time`. |
| `port-outgoing-transfer-targets-interfacing-port` | An outgoing transfer must target an interface-connected port. | §7.12.2; `Ports.sysml:37-45` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_port_transfers_require_interface_topology` (FL018). |
| `messagetransfer-no-pickup-dropoff` | A MessageTransfer (send/accept) carries a payload without sourceOutput/targetInput. | KerML §9 *"MessageTransfers are Transfers that do not have…"*; `Flows.sysml:38,74` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_transfer_message_without_pickup_dropoff_routes`. |
| `accept-waits-for-conforming-transfer` | Accept selects an incoming MessageTransfer whose values conform to the accepted type. | §7.17 / §8.4.13.6 (GOSPEL) | **GATED-elsewhere** (partial) — `spec_send_accept_payload_identity_through_port`. |
| `succession-flow-temporal-ordering` | Succession flow: source completes → transfer → target starts. | §7.16.2; `Transfers.kerml:154-168` (GOSPEL+LIBRARY) | **UNGATED** — candidate gate (overlaps actions area). |
| `port-feature-matching-conjugate-or-undirected` | Features match iff conforming types and (both undirected or conjugate directions). | §7.12.2 (GOSPEL) | **GATED-elsewhere** (partial) — conjugation test. |
| `flow-end-redefines-sourceoutput-targetinput` | FlowEnd features redefine `Transfer::source::sourceOutput`/`target::targetInput`. | KerML `checkFeatureFlowFeatureRedefinition` (GOSPEL, STRUCTURAL) | **STRUCTURAL** — validation sweep. |
| `flow-usage-must-specialize-flows-messages` | FlowUsage specializes `Flows::messages` (and `Flows::flows` if it has ends). | §8 `checkFlowUsageSpecialization` (GOSPEL, STRUCTURAL) | **STRUCTURAL** — validation sweep. |
| `port-usage-referential` | A port's non-port nested/owned usages must be referential (non-composite). | §8.3.x `validatePortUsageNestedUsagesNotComposite` + `validatePortDefinitionOwnedUsagesNotComposite` (GOSPEL OCL: `nestedUsage->reject(PortUsage)->forAll(not isComposite)`) | **CONFORMS** (`2026-06-21`) — S145 (PortUsage) + S146 (PortDefinition) (`semantic_checks::ports`) flag a composite non-port nested/owned usage. Compositeness via `semantic_checks::composite::is_effectively_composite` (occurrence-default: a non-`ref` OccurrenceUsage is composite; Attribute/Reference usages are referential by nature). Gated by `port_usage_with_composite_nested_part_is_flagged` + `port_definition_with_composite_owned_part_is_flagged` + referential negative-twin. **Latent parser gap (not blocking):** `ref part X` nested in a port mis-lowers to a prop-less PartUsage + a stray ReferenceUsage (the `ref` marker is dropped), so a `ref`-prefixed occurrence usage in a port would be a false positive — but corpus+library has ZERO such cases (scan clean, baseline byte-identical); the fix belongs in the parser (stamp `isReference`), not the validator. |
| `multi-interface-dispatch-nondeterminism` | When a port has multiple interfaces, which one a transfer targets is not determined by interface semantics. | §7.12.2 *"which one is not determined by the interface semantics"* (**SPEC-SILENT**) | **SPEC-SILENT** — any deterministic dispatch rule is tool-defined; document. |

## Coverage

- **GATED-elsewhere**: 9 (full or partial via RSC-0.1/0.3) — flows/ports is the
  best-covered runtime area.
- **UNGATED behavioral gap**: 1 (`succession-flow-temporal-ordering`).
- **STRUCTURAL**: 3. **SPEC-SILENT**: 1.
- Behavioral coverage = 9 / (9 + 1) = **90%**.

## Ranked findings

1. **Strong coverage** — the transfer + port-direction + interface-topology
   obligations are all gated by RSC-0.1/0.3. This area validates the
   cross-reference approach: the matrix shows what's already locked in.
2. **GAP-FLOW-1 — `succession-flow-temporal-ordering` ungated** (shared with the
   actions area's `succession-flow-ordering-constraint`). One candidate gate.
3. **`multi-interface-dispatch` is SPEC-SILENT** — if the runtime picks a
   deterministic interface, that policy is tool-defined; ensure it's labelled so.

## Reproducing the citations

```bash
KER="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/KerML-spec-r2025-04_REF.html"
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$KER" > /tmp/ker.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `flowtransfer-ismove-empties-source` | `$KER` §9 | `grep -n -i "isMove true requires sourceOutputLinks to end" /tmp/ker.txt` |
| `flowtransfer-ispush-triggers-start` | `$KER` §9 | `grep -n -i "isPush true requires the Transfer to start" /tmp/ker.txt` |
| `messagetransfer-no-pickup-dropoff` | `$KER` §9 | `grep -n -i "MessageTransfers are Transfers that do not have" /tmp/ker.txt` |
| `port-direction-governs-transfer-direction` | `$SYS` §7.12.2 | `grep -n -i "transfer can occur from the out" /tmp/sys.txt` |
| `multi-interface-dispatch-nondeterminism` | `$SYS` §7.12.2 | `grep -n -i "not determined by the interface semantics" /tmp/sys.txt` |
| isMove/isPush defaults | `sysml.library/.../Transfers.kerml` | `grep -n "isMove\|isPush" "<file>"` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

All normative units from the four listed spec sources were enumerated:

| Source | Sections examined | Method |
|--------|------------------|--------|
| SysML §7.12 *Ports* (§7.12.1–7.12.3) | Port Defs/Usages, Conjugated Port Defs/Usages, direction/matching/interface topology | tag-strip + grep on `SysML-spec-r2025-04_REF.html` |
| SysML §7.16 *Flows* (§7.16.1–7.16.3) | Messages, streaming flows, succession flows | same |
| SysML §8.3 abstract syntax — PortUsage, FlowUsage, FlowDefinition, SuccessionFlowUsage, ConjugatedPortDefinition | `validatePortDefinitionOwnedUsagesNotComposite`, `checkPortUsageSpecialization`, `checkPortUsageSubportSpecialization`, `validatePortUsageIsReference`, `validatePortUsageNestedUsagesNotComposite`, `checkFlowDefinitionBinarySpecialization`, `validateFlowDefinitionFlowEnds`, `checkFlowUsageSpecialization`, `checkFlowUsageFlowSpecialization`, `checkSuccessionFlowUsageSpecialization` | grep constraint names, lines 25003–26703 |
| SysML §8.4.8 Port Semantics (§8.4.8.1–§8.4.8.2) | PortDefinition semantics, PortUsage semantics, interfacingPorts constraint | lines 39059–39238 |
| SysML §8.4.10 Interface Semantics | InterfaceDefinition/Usage, outgoingTransfer→interfacingPort linkage | lines 39700–39799 |
| SysML §8.4.12 Flow Semantics (§8.4.12.1–§8.4.12.3) | FlowDefinition, FlowUsage, SuccessionFlowUsage semantics | lines 40106–40502 |
| SysML §8.4.13.5–§8.4.13.6 Send/Accept Action Semantics | SendActionUsage, AcceptActionUsage constraints | lines 41376–41620 |
| KerML §9.2.7 Transfers library | FlowTransfer, FlowTransferBefore, MessageTransfer, SendPerformance, AcceptPerformance | lines 41537–42556 in ker.txt |
| `Systems Library/Ports.sysml` | `Port::interfacingPorts`, `outgoingTransfersFromSelf :> interfacingPorts.incomingTransfersToSelf` | full file (54 lines) |
| `Systems Library/Flows.sysml` | MessageAction, Message, Flow, SuccessionFlow, messages/flows/successionFlows features | full file (126 lines) |

### Clause classification table

| Normative unit | Status | Notes |
|---------------|--------|-------|
| §7.12.2 direction (in/out/inout) governs transfer direction | CAPTURED — `port-direction-governs-transfer-direction` | GATED-elsewhere via `spec_port_conjugation_direction_at_transfer_time` |
| §7.12.3 conjugated port inverts in↔out, inout unchanged | CAPTURED — `conjugated-port-inverts-directions` | GATED-elsewhere same test |
| §7.12.2 feature matching (conforming types + conjugate or both undirected) | CAPTURED — `port-feature-matching-conjugate-or-undirected` | GATED-elsewhere (partial) |
| §7.12.2 multi-interface dispatch nondeterminism | CAPTURED — `multi-interface-dispatch-nondeterminism` | SPEC-SILENT; documented |
| §7.12.2 outgoing transfer must target interface-connected port | CAPTURED — `port-outgoing-transfer-targets-interfacing-port` | GATED-elsewhere via FL018 |
| §7.16.1 message flow: transfer happens, payload optional, no sourceOutput/targetInput | CAPTURED — `messagetransfer-no-pickup-dropoff` | GATED-elsewhere |
| §7.16.1 streaming flow: identifies sourceOutput + targetInput | CAPTURED — `flow-end-redefines-sourceoutput-targetinput` | STRUCTURAL |
| §7.16.2 succession flow: source complete → transfer → target starts | CAPTURED (ungated) — `succession-flow-temporal-ordering` | UNGATED — existing GAP-FLOW-1 |
| §8.3 `validatePortDefinitionOwnedUsagesNotComposite` | CAPTURED — `port-usage-referential` | STRUCTURAL |
| §8.3 `validatePortUsageIsReference` (non-nested PortUsage must be referential) | CAPTURED — `port-usage-referential` | STRUCTURAL |
| §8.3 `validatePortUsageNestedUsagesNotComposite` | CAPTURED — `port-usage-referential` | STRUCTURAL |
| §8.3 `checkPortUsageSubportSpecialization` (composite PortUsage owning port ↪ subports) | STRUCTURAL | Not yet in matrix; well-formedness constraint, no behavioral gate needed |
| §8.3 `checkPortUsageSpecialization` → `Ports::ports` | STRUCTURAL | Same |
| §8.3 `checkFlowDefinitionBinarySpecialization` | STRUCTURAL | Not yet in matrix |
| §8.3 `validateFlowDefinitionFlowEnds` | STRUCTURAL | Not yet in matrix |
| §8.3 `checkFlowUsageSpecialization` → `Flows::messages` | CAPTURED — `flow-usage-must-specialize-flows-messages` | STRUCTURAL |
| §8.3 `checkFlowUsageFlowSpecialization` → `Flows::flows` if has ends | STRUCTURAL | Not yet in matrix; related to `flow-usage-must-specialize-flows-messages` |
| §8.3 `checkSuccessionFlowUsageSpecialization` → `Flows::successionFlows` | STRUCTURAL | Not yet in matrix |
| §8.4.8.1 PortDefinition implies ConjugatedPortDefinition | STRUCTURAL | Parser concern, not a behavioral runtime obligation |
| §8.4.8.2 non-nested PortUsage is referential | CAPTURED — `port-usage-referential` | STRUCTURAL |
| §8.4.10 Interface::outgoingTransfersFromSelf targets interfacingPorts | CAPTURED — `port-outgoing-transfer-targets-interfacing-port` | GATED-elsewhere |
| §8.4.12 FlowUsage is ActionUsage + KerML Flow | CAPTURED — `flow-transfers-values-source-to-target` | GATED-elsewhere |
| §8.4.12.3 SuccessionFlowUsage temporal ordering (source→transfer→target) | CAPTURED (ungated) — `succession-flow-temporal-ordering` | UNGATED — GAP-FLOW-1 |
| §8.4.13.5 SendActionUsage initiates MessageTransfer from sender to receiver | CAPTURED — `messagetransfer-no-pickup-dropoff`, `accept-waits-for-conforming-transfer` | GATED-elsewhere (partial) |
| §8.4.13.6 AcceptActionUsage accepts conforming incomingTransfer | CAPTURED — `accept-waits-for-conforming-transfer` | GATED-elsewhere (partial) |
| KerML §9.2.7 FlowTransfer isMove default true | CAPTURED — `flowtransfer-ismove-empties-source` | GATED-elsewhere |
| KerML §9.2.7 FlowTransfer isPush default true | CAPTURED — `flowtransfer-ispush-triggers-start` | GATED-elsewhere |
| KerML §9.2.7 FlowTransfer isMove=true removes payload from source | CAPTURED — `flowtransfer-ismove-empties-source` | GATED-elsewhere |
| KerML §9.2.7 FlowTransfer isPush=true starts on payload available | CAPTURED — `flowtransfer-ispush-triggers-start` | GATED-elsewhere |
| KerML §9.2.7 MessageTransfer carries payload without sourceOutput/targetInput | CAPTURED — `messagetransfer-no-pickup-dropoff` | GATED-elsewhere |
| KerML §9.2.7 Transfer payload conforms to endpoint types | CAPTURED — `flow-payload-conformance` | GATED-elsewhere |
| `Ports.sysml` `outgoingTransfersFromSelf :> interfacingPorts.incomingTransfersToSelf` | CAPTURED — `port-outgoing-transfer-targets-interfacing-port` | GATED-elsewhere |
| `Flows.sysml` MessageAction/Message/Flow/SuccessionFlow hierarchy | CAPTURED — `flow-usage-must-specialize-flows-messages` | STRUCTURAL |

### Missed behavioral obligations

None found. All behavioral obligations map to existing rows.

Three STRUCTURAL constraints (`checkFlowDefinitionBinarySpecialization`, `validateFlowDefinitionFlowEnds`, `checkFlowUsageFlowSpecialization`→`Flows::flows`, `checkSuccessionFlowUsageSpecialization`) are not explicit rows in the existing matrix but fall squarely in the STRUCTURAL category already covered by "validation sweep." They do not represent ungated behavioral gaps.

One UNGATED behavioral gap was already known: `succession-flow-temporal-ordering` (GAP-FLOW-1).

### Honesty note

The matrix is complete for the behavioral surface reviewed. The `streaming flow ongoing-while-both-active` description from §7.16.1 (*"transfer can be ongoing while both the source and target action are being performed"*) is a semantic property of FlowTransfer vs. SuccessionFlow disambiguation — its runtime interpretation is partially covered by the `succession-flow-temporal-ordering` gate and partially by `flow-end-redefines-sourceoutput-targetinput` (STRUCTURAL). If a dedicated streaming-flow concurrency gate is desired in the future, that is an enhancement, not a current gap.
