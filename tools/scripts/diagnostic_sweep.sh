#!/usr/bin/env bash
set -euo pipefail

# diagnostic_sweep.sh — Run sysml inspect --diagnostics --json on all example
# collections and compare against a JSON baseline.
#
# Usage:
#   ./scripts/diagnostic_sweep.sh              # Check against baseline
#   ./scripts/diagnostic_sweep.sh --update     # Regenerate baseline
#   ./scripts/diagnostic_sweep.sh --quick      # Only the-book examples (< 30s)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE="$SCRIPT_DIR/baselines/diagnostic_sweep.json"

# CLI binary
SYSML_BIN="$WORKSPACE/target/release/sysml"

# Color helpers
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

# ---------- collection paths (relative to workspace) ----------
declare -A COLLECTION_PATHS
COLLECTION_PATHS[book-coffee]="../the-book/examples/coffee-machine"
COLLECTION_PATHS[book-beverage]="../the-book/examples/beverage-workspace"
COLLECTION_PATHS[cli-fixtures]="sysml-cli/fixtures"
COLLECTION_PATHS[lsp-valid]="sysml-lsp-server/fixtures/valid"

# Collections that --quick includes
QUICK_COLLECTIONS=("book-coffee" "book-beverage")
ALL_COLLECTIONS=("book-coffee" "book-beverage" "cli-fixtures" "lsp-valid")

# ---------- helpers ----------

ensure_binary() {
    if [[ ! -x "$SYSML_BIN" ]]; then
        echo -e "${YELLOW}CLI binary not found, building...${NC}"
        (cd "$WORKSPACE" && cargo build --release -p sysml-cli)
    fi
}

# Collect .sysml files for a collection
collect_files() {
    local coll="$1"
    local base_path="$WORKSPACE/${COLLECTION_PATHS[$coll]}"
    find "$base_path" -name '*.sysml' -type f 2>/dev/null | sort
}

# Run diagnostics on a single file, return JSON array (empty array if no diagnostics)
run_diagnostics() {
    local file="$1"
    local output
    output=$("$SYSML_BIN" inspect --diagnostics --json "$file" 2>/dev/null) || true
    if [[ -z "$output" ]]; then
        echo "[]"
    else
        echo "$output"
    fi
}

# Extract diagnostic keys from a JSON array: code:severity pairs
# Diagnostics without a code field use "NOCODE" as placeholder
extract_keys() {
    local json="$1"
    echo "$json" | python3 -c "
import json, sys
diags = json.load(sys.stdin)
keys = []
for d in diags:
    code = d.get('code', 'NOCODE')
    sev = d.get('severity', 'unknown')
    keys.append(f'{code}:{sev}')
keys.sort()
print(json.dumps(keys))
"
}

# ---------- update mode ----------

do_update() {
    local collections=("${@}")
    echo -e "${BOLD}Generating diagnostic baseline...${NC}"
    ensure_binary

    local result="{}"
    local total_files=0
    local total_diags=0

    for coll in "${collections[@]}"; do
        echo -e "  Scanning ${BOLD}$coll${NC}..."
        local coll_files=()
        local coll_diags="{}"

        while IFS= read -r file; do
            local fname
            fname=$(basename "$file")
            coll_files+=("$fname")

            local diag_json
            diag_json=$(run_diagnostics "$file")

            local keys
            keys=$(extract_keys "$diag_json")

            coll_diags=$(echo "$coll_diags" | python3 -c "
import json, sys
d = json.load(sys.stdin)
d['$fname'] = json.loads('$keys')
print(json.dumps(d))
")
            local count
            count=$(echo "$keys" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))")
            total_diags=$((total_diags + count))
            total_files=$((total_files + 1))
        done < <(collect_files "$coll")

        # Build collection entry
        local files_json
        files_json=$(printf '%s\n' "${coll_files[@]}" | python3 -c "
import json, sys
print(json.dumps([line.strip() for line in sys.stdin if line.strip()]))
")

        result=$(python3 -c "
import json, sys
r = json.loads(sys.argv[1])
r.setdefault('collections', {})[sys.argv[2]] = {
    'files': json.loads(sys.argv[3]),
    'diagnostics': json.loads(sys.argv[4])
}
print(json.dumps(r))
" "$result" "$coll" "$files_json" "$coll_diags")
    done

    # Add timestamp
    result=$(python3 -c "
import json, sys, datetime
r = json.loads(sys.argv[1])
r['generated'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
print(json.dumps(r, indent=2, sort_keys=False))
" "$result")

    mkdir -p "$(dirname "$BASELINE")"
    echo "$result" > "$BASELINE"
    echo ""
    echo -e "${GREEN}Baseline written to $BASELINE${NC}"
    echo -e "  Files scanned: $total_files"
    echo -e "  Total diagnostic keys: $total_diags"
}

# ---------- check mode ----------

do_check() {
    local collections=("${@}")

    if [[ ! -f "$BASELINE" ]]; then
        echo -e "${RED}Baseline not found at $BASELINE${NC}"
        echo "Run with --update to generate the baseline first:"
        echo "  ./scripts/diagnostic_sweep.sh --update"
        exit 2
    fi

    ensure_binary

    echo -e "${BOLD}Diagnostic Sweep — checking against baseline${NC}"
    echo ""

    local total_files=0
    local passed=0
    local regressions=0
    local improvements=0
    local regression_details=()

    for coll in "${collections[@]}"; do
        echo -e "  ${BOLD}$coll${NC}"

        while IFS= read -r file; do
            local fname
            fname=$(basename "$file")
            total_files=$((total_files + 1))

            local diag_json
            diag_json=$(run_diagnostics "$file")
            local current_keys
            current_keys=$(extract_keys "$diag_json")

            # Get baseline keys for this file
            local baseline_keys
            baseline_keys=$(python3 -c "
import json, sys
baseline = json.load(open(sys.argv[1]))
coll_data = baseline.get('collections', {}).get(sys.argv[2], {})
diags = coll_data.get('diagnostics', {})
keys = diags.get(sys.argv[3], [])
print(json.dumps(keys))
" "$BASELINE" "$coll" "$fname")

            # Compare: find new regressions (in current but not in baseline)
            local diff_result
            diff_result=$(python3 -c "
import json, sys
current = json.loads(sys.argv[1])
baseline = json.loads(sys.argv[2])
new_issues = sorted(set(current) - set(baseline))
fixed = sorted(set(baseline) - set(current))
print(json.dumps({'new': new_issues, 'fixed': fixed}))
" "$current_keys" "$baseline_keys")

            local new_count fixed_count
            new_count=$(echo "$diff_result" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['new']))")
            fixed_count=$(echo "$diff_result" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['fixed']))")

            if [[ "$new_count" -gt 0 ]]; then
                echo -e "    ${RED}REGRESSION${NC} $fname (+$new_count new diagnostics)"
                local new_items
                new_items=$(echo "$diff_result" | python3 -c "import json,sys; [print(f'      - {k}') for k in json.load(sys.stdin)['new']]")
                echo "$new_items"
                regressions=$((regressions + new_count))
                regression_details+=("$coll/$fname: +$new_count")
            elif [[ "$fixed_count" -gt 0 ]]; then
                echo -e "    ${GREEN}IMPROVED${NC}   $fname (-$fixed_count diagnostics)"
                improvements=$((improvements + fixed_count))
                passed=$((passed + 1))
            else
                echo -e "    ${GREEN}PASS${NC}       $fname"
                passed=$((passed + 1))
            fi
        done < <(collect_files "$coll")
    done

    echo ""
    echo -e "${BOLD}Summary${NC}"
    echo "  Total files:    $total_files"
    echo -e "  Passed:         ${GREEN}$passed${NC}"
    if [[ "$improvements" -gt 0 ]]; then
        echo -e "  Improvements:   ${GREEN}$improvements fewer diagnostics${NC}"
    fi
    if [[ "$regressions" -gt 0 ]]; then
        echo -e "  Regressions:    ${RED}$regressions new diagnostics${NC}"
        echo ""
        echo -e "${RED}FAIL — new regressions detected:${NC}"
        for detail in "${regression_details[@]}"; do
            echo "  - $detail"
        done
        exit 1
    else
        echo ""
        echo -e "${GREEN}PASS — no regressions${NC}"
        exit 0
    fi
}

# ---------- main ----------

MODE="check"
case "${1:-}" in
    --update) MODE="update" ;;
    --quick)  MODE="quick" ;;
    --help|-h)
        echo "Usage:"
        echo "  ./scripts/diagnostic_sweep.sh              # Check against baseline"
        echo "  ./scripts/diagnostic_sweep.sh --update     # Regenerate baseline"
        echo "  ./scripts/diagnostic_sweep.sh --quick      # Only the-book examples (< 30s)"
        exit 0
        ;;
esac

case "$MODE" in
    update) do_update "${ALL_COLLECTIONS[@]}" ;;
    quick)  do_check "${QUICK_COLLECTIONS[@]}" ;;
    check)  do_check "${ALL_COLLECTIONS[@]}" ;;
esac
