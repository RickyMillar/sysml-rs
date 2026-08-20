#!/usr/bin/env bash
set -euo pipefail

# severity_audit.sh — Check that diagnostic codes emit the expected severity.
#
# Maintains a hardcoded expected-severity table. Runs diagnostics on all
# collections and flags any code whose observed severity doesn't match.
#
# Usage:
#   ./scripts/severity_audit.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

# CLI binary
SYSML_BIN="$WORKSPACE/target/release/sysml"

# Color helpers
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

# ---------- expected severity table ----------
declare -A EXPECTED_SEVERITY

# AX — Action/expression diagnostics
EXPECTED_SEVERITY[AX001]="error"
EXPECTED_SEVERITY[AX002]="warning"
EXPECTED_SEVERITY[AX003]="error"
EXPECTED_SEVERITY[AX004]="warning"
EXPECTED_SEVERITY[AX005]="warning"
EXPECTED_SEVERITY[AX006]="error"
EXPECTED_SEVERITY[AX007]="error"
EXPECTED_SEVERITY[AX008]="error"
EXPECTED_SEVERITY[AX009]="error"
EXPECTED_SEVERITY[AX010]="warning"
EXPECTED_SEVERITY[AX011]="warning"
EXPECTED_SEVERITY[AX012]="warning"
EXPECTED_SEVERITY[AX013]="warning"

# FL — Flow diagnostics
EXPECTED_SEVERITY[FL001]="error"
EXPECTED_SEVERITY[FL002]="error"
EXPECTED_SEVERITY[FL003]="warning"
EXPECTED_SEVERITY[FL004]="warning"
EXPECTED_SEVERITY[FL005]="warning"
EXPECTED_SEVERITY[FL006]="info"
EXPECTED_SEVERITY[FL007]="warning"
EXPECTED_SEVERITY[FL008]="info"
EXPECTED_SEVERITY[FL009]="warning"

# IM — Import/membership diagnostics
EXPECTED_SEVERITY[IM001]="info"
EXPECTED_SEVERITY[IM002]="warning"
EXPECTED_SEVERITY[IM003]="info"
EXPECTED_SEVERITY[IM004]="error"
EXPECTED_SEVERITY[IM005]="info"
EXPECTED_SEVERITY[IM006]="error"

# SM — State machine diagnostics
EXPECTED_SEVERITY[SM001]="error"
EXPECTED_SEVERITY[SM002]="warning"
EXPECTED_SEVERITY[SM003]="warning"
EXPECTED_SEVERITY[SM004]="warning"
EXPECTED_SEVERITY[SM005]="warning"
EXPECTED_SEVERITY[SM006]="error"
EXPECTED_SEVERITY[SM007]="error"

# VC — Validation/constraint diagnostics
EXPECTED_SEVERITY[VC001]="error"
EXPECTED_SEVERITY[VC002]="error"
EXPECTED_SEVERITY[VC003]="warning"
EXPECTED_SEVERITY[VC004]="warning"
EXPECTED_SEVERITY[VC005]="warning"
EXPECTED_SEVERITY[VC006]="info"
EXPECTED_SEVERITY[VC007]="warning"
EXPECTED_SEVERITY[VC008]="info"
EXPECTED_SEVERITY[VC009]="warning"
EXPECTED_SEVERITY[VC010]="error"

# E — Error diagnostics
EXPECTED_SEVERITY[E004]="error"
EXPECTED_SEVERITY[E200]="error"

# S — Structural/semantic validation diagnostics (codegen rules)
EXPECTED_SEVERITY[S001]="error"
EXPECTED_SEVERITY[S011]="error"
EXPECTED_SEVERITY[S031]="error"
EXPECTED_SEVERITY[S126]="error"

# V — Validation diagnostics (V001 is codegen rule, V002-V005 are property validation)
EXPECTED_SEVERITY[V001]="warning"
EXPECTED_SEVERITY[V002]="warning"
EXPECTED_SEVERITY[V003]="warning"
EXPECTED_SEVERITY[V004]="warning"
EXPECTED_SEVERITY[V005]="warning"

# ---------- collection paths ----------
declare -A COLLECTION_PATHS
COLLECTION_PATHS[book-coffee]="../the-book/examples/coffee-machine"
COLLECTION_PATHS[book-beverage]="../the-book/examples/beverage-workspace"
COLLECTION_PATHS[cli-fixtures]="sysml-cli/fixtures"
COLLECTION_PATHS[lsp-valid]="sysml-lsp-server/fixtures/valid"

ALL_COLLECTIONS=("book-coffee" "book-beverage" "cli-fixtures" "lsp-valid")

# ---------- helpers ----------

ensure_binary() {
    if [[ ! -x "$SYSML_BIN" ]]; then
        echo -e "${YELLOW}CLI binary not found, building...${NC}"
        (cd "$WORKSPACE" && cargo build --release -p sysml-cli)
    fi
}

collect_files() {
    local coll="$1"
    local base_path="$WORKSPACE/${COLLECTION_PATHS[$coll]}"
    find "$base_path" -name '*.sysml' -type f 2>/dev/null | sort
}

# ---------- main ----------

ensure_binary

echo -e "${BOLD}Severity Audit${NC}"
echo "==============="
echo ""
echo "Expected severity table (${#EXPECTED_SEVERITY[@]} codes):"
for code in $(echo "${!EXPECTED_SEVERITY[@]}" | tr ' ' '\n' | sort); do
    echo "  $code -> ${EXPECTED_SEVERITY[$code]}"
done
echo ""

violations=0
total_checked=0
codes_seen=()

for coll in "${ALL_COLLECTIONS[@]}"; do
    echo -e "${BOLD}$coll${NC}"

    while IFS= read -r file; do
        fname=$(basename "$file")

        # Get diagnostics JSON
        diag_json=$("$SYSML_BIN" inspect --diagnostics --json "$file" 2>/dev/null) || true
        if [[ -z "$diag_json" ]]; then
            diag_json="[]"
        fi

        # Check each diagnostic with a code against the table
        while IFS='|' read -r code severity; do
            [[ -z "$code" ]] && continue

            total_checked=$((total_checked + 1))
            codes_seen+=("$code")

            if [[ -n "${EXPECTED_SEVERITY[$code]+x}" ]]; then
                expected="${EXPECTED_SEVERITY[$code]}"
                if [[ "$severity" != "$expected" ]]; then
                    echo -e "  ${RED}VIOLATION${NC} $fname: $code is ${YELLOW}$severity${NC}, expected ${GREEN}$expected${NC}"
                    violations=$((violations + 1))
                fi
            fi
        done < <(echo "$diag_json" | python3 -c "
import json, sys
diags = json.load(sys.stdin)
for d in diags:
    code = d.get('code', '')
    sev = d.get('severity', '')
    if code:
        print(f'{code}|{sev}')
")
    done < <(collect_files "$coll")
done

echo ""
echo -e "${BOLD}Summary${NC}"
echo "  Diagnostic instances checked: $total_checked"

# Report which expected codes were never seen
unseen=()
for code in $(echo "${!EXPECTED_SEVERITY[@]}" | tr ' ' '\n' | sort); do
    found=0
    for seen in "${codes_seen[@]+"${codes_seen[@]}"}"; do
        if [[ "$seen" == "$code" ]]; then
            found=1
            break
        fi
    done
    if [[ "$found" -eq 0 ]]; then
        unseen+=("$code")
    fi
done

if [[ ${#unseen[@]} -gt 0 ]]; then
    echo -e "  ${YELLOW}Codes in table but not observed:${NC} ${unseen[*]}"
fi

echo ""
if [[ "$violations" -gt 0 ]]; then
    echo -e "${RED}FAIL — $violations severity violation(s)${NC}"
    exit 1
else
    echo -e "${GREEN}PASS — all observed diagnostics match expected severity${NC}"
    exit 0
fi
