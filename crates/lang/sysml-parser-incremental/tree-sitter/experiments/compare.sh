#!/bin/bash
# Compare tree-sitter grammar optimization experiment results
#
# Reads all experiments/results/*.json files and prints a comparison table.
#
# Usage:
#   ./compare.sh             # Compare all experiments
#   ./compare.sh --json      # Output raw JSON array
#   ./compare.sh --csv       # Output CSV format

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results"

FORMAT="table"
if [[ "${1:-}" == "--json" ]]; then FORMAT="json"; fi
if [[ "${1:-}" == "--csv" ]]; then FORMAT="csv"; fi

if [[ ! -d "$RESULTS_DIR" ]] || [[ -z "$(ls "$RESULTS_DIR"/*.json 2>/dev/null)" ]]; then
    echo "No experiment results found in $RESULTS_DIR/" >&2
    echo "Run ./bench.sh baseline first to capture baseline metrics." >&2
    exit 1
fi

# Read all results with python3 (available on all modern systems)
python3 - "$RESULTS_DIR" "$FORMAT" <<'PYEOF'
import json, sys, os, glob

results_dir = sys.argv[1]
fmt = sys.argv[2]

results = []
for f in sorted(glob.glob(os.path.join(results_dir, "*.json"))):
    try:
        with open(f) as fh:
            results.append(json.load(fh))
    except (json.JSONDecodeError, IOError) as e:
        print(f"WARNING: skipping {f}: {e}", file=sys.stderr)

if not results:
    print("No valid result files found.", file=sys.stderr)
    sys.exit(1)

# Sort: baseline first, then alphabetical
results.sort(key=lambda r: ("" if r["name"] == "baseline" else r["name"]))

# Find baseline for delta calculation
baseline = next((r for r in results if r["name"] == "baseline"), None)

def fmt_size(b):
    if b == 0: return "N/A"
    return f"{b / 1_048_576:.1f} MB"

def fmt_time(s):
    if s == 0: return "N/A"
    return f"{s // 60}m {s % 60:02d}s"

def fmt_delta(current, base):
    if base == 0 or current == 0: return "---"
    pct = (current - base) / base * 100
    sign = "+" if pct > 0 else ""
    return f"{sign}{pct:.1f}%"

if fmt == "json":
    print(json.dumps(results, indent=2))
    sys.exit(0)

if fmt == "csv":
    print("name,parser_c_bytes,state_count,large_state_count,generate_time_sec,corpus_pass,corpus_total,library_pass,library_total,exit_code")
    for r in results:
        print(f"{r['name']},{r['parser_c_bytes']},{r['state_count']},{r['large_state_count']},{r['generate_time_sec']},{r['corpus_pass']},{r['corpus_total']},{r['library_pass']},{r['library_total']},{r['exit_code']}")
    sys.exit(0)

# Table format
header = f"{'EXPERIMENT':<28} {'SIZE':>8} {'STATES':>7} {'LARGE':>7} {'GEN TIME':>10} {'CORPUS':>9} {'LIBRARY':>9} {'DELTA':>8}"
sep    = "-" * len(header)

print()
print("Tree-sitter Grammar Optimization Experiments")
print(sep)
print(header)
print(sep)

for r in results:
    name = r["name"]
    size = fmt_size(r["parser_c_bytes"])
    states = str(r["state_count"]) if r["state_count"] else "N/A"
    large = str(r["large_state_count"]) if r["large_state_count"] else "N/A"
    time = fmt_time(r["generate_time_sec"])
    corpus = f"{r['corpus_pass']}/{r['corpus_total']}"
    library = f"{r['library_pass']}/{r['library_total']}"

    if baseline and r["name"] != "baseline":
        delta = fmt_delta(r["parser_c_bytes"], baseline["parser_c_bytes"])
    else:
        delta = "---"

    if r.get("exit_code", 0) != 0:
        delta = "FAILED"

    print(f"{name:<28} {size:>8} {states:>7} {large:>7} {time:>10} {corpus:>9} {library:>9} {delta:>8}")

print(sep)

if baseline:
    print(f"\nBaseline: {baseline['name']} ({baseline['timestamp']})")
    print(f"  {fmt_size(baseline['parser_c_bytes'])}, {baseline['state_count']} states, {fmt_time(baseline['generate_time_sec'])}")

print()
PYEOF
