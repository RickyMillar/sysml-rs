#!/usr/bin/env bash
# scripts/run-gates.sh — the standing gate set for sysml-rs runtime/semantics arcs.
#
# Every runtime-arc commit message should cite a run of this script (see
# regression (98063730) shipped without being caught: the byte-identical
# baseline oracles in rsc4_b0_baselines are #[ignore]'d (they step long
# trajectories) so plain `cargo test` never runs them, and a hand-rolled gate
# script assumed rsc2_behavioural_baseline lived in sysml-runtime when it
# actually lives in sysml-spec-tests. Both are discoverability failures —
# this script is the single source of truth so nobody has to rediscover the
# invocations by hand again.
#
# Usage:
#   scripts/run-gates.sh            # --quick (default)
#   scripts/run-gates.sh --quick    # fast standing gates only
#   scripts/run-gates.sh --full     # --quick + full crate test suites
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MODE="${1:---quick}"
case "$MODE" in
  --quick|--full) ;;
  *)
    echo "usage: $0 [--quick|--full]" >&2
    exit 2
    ;;
esac

# ---------------------------------------------------------------------------
# KNOWN-RED: tests currently failing for a tracked, understood reason.
#
# A KNOWN-RED gate is reported loudly as "KNOWN-RED (tracked)" — never as a
# silent skip, and never folded into a plain PASS. If a gate fails and every
# one of its failing test names is in this map, the script explains why and
# keeps going. If ANY failing test is *not* in this map, that's a real
# regression and the script fails loud.
#
# Remove an entry the moment its tracked fix lands and the test goes green —
# this list is a ledger of open debt, not a place to park inconvenient reds.
# ---------------------------------------------------------------------------
declare -A KNOWN_RED=(
  # Currently empty: all previously-parked reds (WS-D re-blessed baselines and
  # the task-#8 rsc2_behavioural_baseline phantom-subsystem entries) have been
  # resolved and their entries removed. Add a row only for a genuinely-tracked,
  # temporarily-red gate, with the task id and reason.
)

# project_discovery tests that write into ~/.cache/sysml-rs/dependencies and
# fail under this harness's sandboxed shell with "Read-only file system (os
# error 30)" — not a code regression. Only treat as KNOWN-RED when
# SYSML_SANDBOXED_SHELL=1 is set by the caller; anywhere else (CI, an
# unsandboxed shell, dangerouslyDisableSandbox) a failure here is real and
# must fail the gate.
SANDBOX_FS_KNOWN_RED_TESTS=(
  "project_discovery::tests::discover_sysml_project_hydrates_git_and_kpar_dependencies"
  "project_discovery::tests::resolve_manifest_dependencies_hydrates_registry_dependency_when_index_present"
)
if [ "${SYSML_SANDBOXED_SHELL:-0}" = "1" ]; then
  for t in "${SANDBOX_FS_KNOWN_RED_TESTS[@]}"; do
    KNOWN_RED["$t"]="writes into ~/.cache/sysml-rs/dependencies; fails with 'Read-only file system (os error 30)' under a sandboxed shell, not a real regression. Re-run with SYSML_SANDBOXED_SHELL unset (or outside the sandbox) to confirm before trusting a green here."
  done
fi

SUMMARY=()
OVERALL_FAIL=0

# run_gate NAME CMD...
# Runs CMD, classifies the result as PASS / KNOWN-RED (tracked) / FAIL, and
# records it for the closing summary table. Never lets `set -e` abort the
# script on a gate failure — we want every gate to run and the summary to be
# complete even if an early gate is red.
run_gate() {
  local name="$1"; shift
  echo
  echo "=== $name ==="
  echo "\$ $*"
  local out status
  set +e
  out="$("$@" 2>&1)"
  status=$?
  set -e

  if [ "$status" -eq 0 ]; then
    echo "$out" | tail -5
    echo "PASS: $name"
    SUMMARY+=("PASS|$name")
    return 0
  fi

  local last_failures_line failing_block failing unknown=0 reasons=""
  last_failures_line=$(printf '%s\n' "$out" | grep -n '^failures:$' | tail -1 | cut -d: -f1 || true)

  if [ -z "$last_failures_line" ]; then
    # No cargo test summary at all — a build error or panic before the
    # harness printed results. Always a real failure.
    echo "$out" | tail -40
    echo "FAIL: $name (no test summary — build or run error, see output above)"
    SUMMARY+=("FAIL|$name")
    OVERALL_FAIL=1
    # NB: return 0 here, not 1. Every call site is a bare top-level
    # statement under `set -e` — a nonzero return from run_gate itself
    # (as opposed to the gate command it ran, which is insulated above)
    # would abort the whole script on the first red gate, exactly the
    # early-abort bug this function exists to prevent. OVERALL_FAIL is
    # the single source of truth for the final exit code; see the
    # bottom of the script.
    return 0
  fi

  failing_block=$(printf '%s\n' "$out" | tail -n +"$((last_failures_line + 1))")
  failing=$(printf '%s\n' "$failing_block" | awk '/^$/{exit} {print $1}')

  while IFS= read -r t; do
    [ -z "$t" ] && continue
    if [ -n "${KNOWN_RED[$t]+x}" ]; then
      reasons+="    - $t: ${KNOWN_RED[$t]}"$'\n'
    else
      unknown=1
    fi
  done <<< "$failing"

  if [ "$unknown" -eq 0 ] && [ -n "$failing" ]; then
    echo "$out" | tail -15
    echo "KNOWN-RED (tracked): $name"
    printf '%s' "$reasons"
    SUMMARY+=("KNOWN-RED|$name")
    return 0
  fi

  echo "$out" | tail -40
  echo "FAIL: $name"
  SUMMARY+=("FAIL|$name")
  OVERALL_FAIL=1
  # See NB above: return 0, not 1 — run_gate must never abort the script
  # via set -e. OVERALL_FAIL carries the failure to the final exit code.
  return 0
}

# run_gate_release NAME CARGO-ARGS...
# Like run_gate, but forces a `cargo test --release` build. Two things follow
# from --release that are the whole point of this tier:
#   1. artifacts land in target/release, cached independently of the debug
#      gates above — the two build trees never clobber each other, so an
#      already-warm debug `target/` is untouched (and vice versa);
#   2. heavy full-horizon trajectories run at release speed.
# Used for gates whose DEBUG walltime is prohibitive — e.g. a stiff, small-dt
# ODE stepped over hundreds of thousands of steps, which can be minutes per test
# in debug versus ~1-2 min in release. The name is tagged "(release)" so the
# summary table shows which profile each gate ran under. (No release-tier gate
# is currently wired in the public tree; the helper stays for future heavy gates.)
run_gate_release() {
  local name="$1"; shift
  run_gate "$name (release)" cargo test --release "$@"
}

# ---------------------------------------------------------------------------
# --quick tier
# ---------------------------------------------------------------------------
# rsc4_b0_read_set_inventory: RSC-4.1 read-set coverage meter. #[ignore]'d
# because it replays long trajectories — invisible to plain `cargo test`.
run_gate "rsc4_b0_read_set_inventory --ignored" \
  cargo test -p sysml-runtime --test rsc4_b0_read_set_inventory -- --ignored --nocapture

# rsc2_behavioural_baseline: lives in sysml-spec-tests, NOT sysml-runtime —
# this is exactly the crate this script exists to stop people getting wrong.
run_gate "rsc2_behavioural_baseline (sysml-spec-tests)" \
  cargo test -p sysml-spec-tests --test rsc2_behavioural_baseline

# ws_determinism: two builds of the same graph must be byte-identical every
# tick (HashMap-order determinism guard, WS-C).
run_gate "ws_determinism" \
  cargo test -p sysml-runtime --test ws_determinism

# event_fn_no_leak: zero-crossing checks must never mutate the master slot
# store (the write-leak class of bug behind task #10).
run_gate "event_fn_no_leak" \
  cargo test -p sysml-runtime --test event_fn_no_leak

# NOTE: the confidential full-horizon trip-timing suite (device-calibrated IEC
# verdict-matrix + fault-detection-chain gates) was moved to the private fixture
# pack in the retired internal extraction (PURGE-06); it is no longer runnable from the
# public tree. The generic verdict-routing capability it exercised is covered by
# the espresso-pump verification gates in the standard suites below.

# ---------------------------------------------------------------------------
# --full tier (quick + full crate suites)
# ---------------------------------------------------------------------------
if [ "$MODE" = "--full" ]; then
  run_gate "cargo test -p sysml-runtime (full)" \
    cargo test -p sysml-runtime

  # NOTE: two project_discovery tests write into ~/.cache/sysml-rs/dependencies
  # and fail under this harness's sandboxed shell ("Read-only file system, os
  # error 30"), not because of a real regression. Set
  # SYSML_SANDBOXED_SHELL=1 to treat exactly those two as KNOWN-RED; leave
  # unset (default) everywhere else, including CI.
  run_gate "cargo test -p sysml-service (full)" \
    cargo test -p sysml-service

  run_gate "cargo test -p sysml-spec-tests (full)" \
    cargo test -p sysml-spec-tests
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
echo "================ run-gates.sh summary ($MODE) ================"
printf '%-16s %s\n' "STATUS" "GATE"
for entry in "${SUMMARY[@]}"; do
  IFS='|' read -r status name <<< "$entry"
  printf '%-16s %s\n' "$status" "$name"
done
echo "================================================================"

if [ "$OVERALL_FAIL" -ne 0 ]; then
  echo "run-gates.sh: FAILED — one or more gates are red for an untracked reason." >&2
  exit 1
fi

echo "run-gates.sh: all gates PASS or KNOWN-RED (tracked)."
