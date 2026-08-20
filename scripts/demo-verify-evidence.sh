#!/usr/bin/env bash
# demo-verify-evidence.sh — populate the Verify workbench with ALL THREE
# evidence modes + a manual attestation, on examples/espresso-production-cell.
#
#
# What it produces (repeatable; archive state is per-server-lifetime):
#   ∿ trajectory — a real bounded brew run (the espresso-demo-recipe beats),
#     verified live via sessions.verify, then stopped into the archive
#   ↓ external (fresh) — an ingested HIL run whose declared digest MATCHES
#     the current model
#   ↓ external (stale) — an ingested CI run against an OLD digest → the
#     timeline renders the ⚑ "older model" staleness label
#   ✎ attestation — a signed manual act on one case (renders in the
#     process/history surfaces, never as a verdict)
#
# Usage:  ./scripts/demo-verify-evidence.sh   (server on 127.0.0.1:8080)
#   API=http://host:port/api/command ./scripts/demo-verify-evidence.sh
#
# Prereq: a CURRENT sysml-api release binary (rebuild → kill by PID →
# relaunch; a stale binary silently lacks the newer commands).
set -euo pipefail

API=${API:-http://127.0.0.1:8080/api/command}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WS="$ROOT/examples/espresso-production-cell"
ACTOR=${ACTOR:-demo-engineer}

post() { curl -s -m 180 -X POST "$API" -H 'content-type: application/json' -d "$1"; }
jqpy() { python3 -c "import json,sys; d=json.load(sys.stdin); $1"; }

echo "── 1/6 load workspace: $WS"
post '{"command":"sysml.load_workspace","params":{"root":"'"$WS"'"}}' \
  | jqpy 'print("   loaded, errors:", d.get("error_count"))'

echo "── 2/6 live session: create + run a bounded brew window (orchestrator, exact bulk step)"
SID=$(post '{"command":"sysml.sessions.create","params":{"uri":"__workspace__"}}' \
  | jqpy 'print(d["id"])')
echo "   session: $SID"
# Multi-file cell → orchestrator session; step in exact 200-tick bulk beats
# (over-cap MAX_BULK_STEP_TICKS is a hard error, never clamped).
for _ in 1 2 3 4 5; do
  post '{"command":"sysml.sessions.step","params":{"session_id":"'"$SID"'","ticks":200}}' >/dev/null
done

echo "── 3/6 trajectory verdicts: sessions.verify against the live run"
post '{"command":"sysml.sessions.verify","params":{"session_id":"'"$SID"'"}}' \
  | jqpy '
rows = d if isinstance(d, list) else d.get("verdicts", [])
from collections import Counter
tally = Counter(v.get("verdict") for v in rows)
print("   {} case verdicts archived (trajectory):".format(len(rows)),
      ", ".join("{} {}".format(n, k) for k, n in sorted(tally.items())))'
post '{"command":"sysml.sessions.stop","params":{"session_id":"'"$SID"'"}}' >/dev/null
echo "   session stopped → archived (B6 provenance captured at mint)"

echo "── 4/6 external ingest #1 (FRESH digest — hil-bench-2)"
DIGEST=$(post '{"command":"sysml.workspace.verify","params":{}}' | jqpy 'print(d["model_digest"])')
echo "   current model digest: ${DIGEST:0:12}…"
post '{"command":"sysml.verify.record_external","params":{
  "tool":"hil-bench-2",
  "declared_digest":"'"$DIGEST"'",
  "run_ref":"https://ci.example/hil/run/8841",
  "artifacts":["https://ci.example/hil/run/8841/report.html"],
  "label":"HIL bench overnight",
  "verdicts":[{"case_id":"BrewQualityCase","verdict":"pass"}]
}}' | jqpy 'print("   recorded:", d["recorded"], "| matches_current_model:", d["matches_current_model"])'

echo "── 5/6 external ingest #2 (STALE digest — pytest-ci → ⚑ older model)"
post '{"command":"sysml.verify.record_external","params":{
  "tool":"pytest-ci",
  "declared_digest":"demo-stale-digest-0000000000000000",
  "run_ref":"https://ci.example/pytest/run/512",
  "label":"CI regression sweep (pre-refactor model)",
  "verdicts":[{"case_id":"SafetyEnvelopeCase","verdict":"pass"},
              {"case_id":"ThroughputCase","verdict":"inconclusive"}]
}}' | jqpy 'print("   recorded:", d["recorded"], "| matches_current_model:", d["matches_current_model"], " (false = the staleness label)")'

echo "── 6/6 manual attestation (✎ — a signed act, never a verdict)"
CASE_EID=$(post '{"command":"sysml.evaluate.verification_cases","params":{}}' \
  | jqpy 'print([r["element_id"] for r in d if r["case_name"]=="SafetyEnvelopeCase"][0])')
post '{"command":"sysml.workflow.attest_verification","params":{
  "project":"espresso-production-cell",
  "element_id":"'"$CASE_EID"'",
  "method":"demo",
  "statement":"bench demonstration witnessed: boiler ≤ 130 °C and manifold ≤ 20 bar over the run",
  "actor":"'"$ACTOR"'"
}}' | jqpy 'print("   attested by", d.get("actor"), "on", d.get("element_id","")[:8]+"…")'

echo
echo "── verify.timeline now carries all three modes:"
post '{"command":"sysml.verify.timeline","params":{}}' \
  | jqpy '
entries = d.get("entries", [])
glyphs = {"static": "=", "trajectory": "~", "external": "v"}
for e in entries:
    ext = e.get("external") or {}
    tag = glyphs.get(e.get("evaluation_mode"), "?")
    extra = ""
    if ext:
        stale = ext.get("matches_current_model") is False
        extra = " tool={} stale={}".format(ext.get("tool"), stale)
    print("   {} {}: {} [{}]{}".format(tag, e.get("case_id"), e.get("verdict"), e.get("evaluation_mode"), extra))
print("   ({} entries)".format(len(entries)))'

echo
echo "Done. Open /verify on the app (vite :3010) — matrix, case view, and History"
echo "now have trajectory lanes, a fresh + a stale external lane, and an ✎ attestation."
