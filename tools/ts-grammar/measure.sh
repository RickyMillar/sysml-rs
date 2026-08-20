#!/usr/bin/env bash
# measure.sh - Report current tree-sitter grammar metrics from parser.c.
#
# Reads parser.c and extracts the build-time cliff indicators:
#   parser.c size, STATE_COUNT, LARGE_STATE_COUNT, SYMBOL_COUNT,
#   ALIAS_COUNT, TOKEN_COUNT.
#
# If .tree-sitter-cache/baseline-metrics.json exists, compares to baseline
# and flags any metric that grew >10% as a build-time-cliff warning.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

PARSER_C="$REPO_ROOT/crates/lang/sysml-parser-incremental/tree-sitter/src/parser.c"
BASELINE="$REPO_ROOT/.tree-sitter-cache/baseline-metrics.json"

CAPTURE_BASELINE="0"
if [[ "${1:-}" == "--capture-baseline" ]]; then
    CAPTURE_BASELINE="1"
fi

if [[ ! -f "$PARSER_C" ]]; then
    echo "error: parser.c not found at $PARSER_C" >&2
    echo "       run tools/ts-grammar/cache-build.sh first" >&2
    exit 1
fi

# Extract a numeric #define value, e.g. extract_define STATE_COUNT.
extract_define() {
    local key="$1"
    grep -m1 "^#define $key " "$PARSER_C" 2>/dev/null \
        | awk '{print $3}' \
        | tr -d '\r'
}

SIZE_BYTES="$(stat -c '%s' "$PARSER_C" 2>/dev/null || stat -f '%z' "$PARSER_C")"
SIZE_MB=$(( SIZE_BYTES / 1024 / 1024 ))

STATE_COUNT="$(extract_define STATE_COUNT)"
LARGE_STATE_COUNT="$(extract_define LARGE_STATE_COUNT)"
SYMBOL_COUNT="$(extract_define SYMBOL_COUNT)"
ALIAS_COUNT="$(extract_define ALIAS_COUNT)"
TOKEN_COUNT="$(extract_define TOKEN_COUNT)"
PRODUCTION_ID_COUNT="$(extract_define PRODUCTION_ID_COUNT)"
FIELD_COUNT="$(extract_define FIELD_COUNT)"
MAX_ALIAS_SEQUENCE_LENGTH="$(extract_define MAX_ALIAS_SEQUENCE_LENGTH)"

printf "tree-sitter grammar metrics\n"
printf "==========================================\n"
printf "%-30s %s\n" "parser.c size (MB)"          "$SIZE_MB"
printf "%-30s %s\n" "STATE_COUNT"                 "${STATE_COUNT:-?}"
printf "%-30s %s\n" "LARGE_STATE_COUNT"           "${LARGE_STATE_COUNT:-?}"
printf "%-30s %s\n" "SYMBOL_COUNT"                "${SYMBOL_COUNT:-?}"
printf "%-30s %s\n" "ALIAS_COUNT"                 "${ALIAS_COUNT:-?}"
printf "%-30s %s\n" "TOKEN_COUNT"                 "${TOKEN_COUNT:-?}"
printf "%-30s %s\n" "PRODUCTION_ID_COUNT"         "${PRODUCTION_ID_COUNT:-?}"
printf "%-30s %s\n" "FIELD_COUNT"                 "${FIELD_COUNT:-?}"
printf "%-30s %s\n" "MAX_ALIAS_SEQUENCE_LENGTH"   "${MAX_ALIAS_SEQUENCE_LENGTH:-?}"
printf "==========================================\n"

write_baseline() {
    cat > "$BASELINE" <<EOF
{
  "captured_at": "$(date -Iseconds)",
  "parser_c_size_mb": $SIZE_MB,
  "parser_c_size_bytes": $SIZE_BYTES,
  "STATE_COUNT": ${STATE_COUNT:-0},
  "LARGE_STATE_COUNT": ${LARGE_STATE_COUNT:-0},
  "SYMBOL_COUNT": ${SYMBOL_COUNT:-0},
  "ALIAS_COUNT": ${ALIAS_COUNT:-0},
  "TOKEN_COUNT": ${TOKEN_COUNT:-0},
  "PRODUCTION_ID_COUNT": ${PRODUCTION_ID_COUNT:-0},
  "FIELD_COUNT": ${FIELD_COUNT:-0},
  "MAX_ALIAS_SEQUENCE_LENGTH": ${MAX_ALIAS_SEQUENCE_LENGTH:-0}
}
EOF
    echo ""
    echo "baseline written: $BASELINE"
}

if [[ "$CAPTURE_BASELINE" == "1" ]]; then
    mkdir -p "$(dirname "$BASELINE")"
    write_baseline
    exit 0
fi

if [[ -f "$BASELINE" ]]; then
    echo ""
    echo "comparing to baseline ($BASELINE):"

    # Extract a numeric field from the baseline JSON without requiring jq.
    baseline_field() {
        grep -m1 "\"$1\"" "$BASELINE" | sed -E 's/.*: *([0-9]+).*/\1/'
    }

    compare() {
        local label="$1" current="$2" baseline="$3"
        if [[ -z "$baseline" || "$baseline" == "0" ]]; then
            printf "  %-30s current=%s baseline=(none)\n" "$label" "$current"
            return
        fi
        # Integer-only delta math: pct = (current - baseline) * 100 / baseline.
        local delta=$(( current - baseline ))
        local pct=$(( delta * 100 / baseline ))
        local warn=""
        if [[ $pct -ge 10 ]]; then
            warn="  <-- WARN: +${pct}% (build-time cliff risk)"
        elif [[ $pct -le -10 ]]; then
            warn="  (good: ${pct}%)"
        fi
        printf "  %-30s %s -> %s (%+d%%)%s\n" "$label" "$baseline" "$current" "$pct" "$warn"
    }

    compare "parser.c size (MB)"        "$SIZE_MB"             "$(baseline_field parser_c_size_mb)"
    compare "STATE_COUNT"               "${STATE_COUNT:-0}"    "$(baseline_field STATE_COUNT)"
    compare "LARGE_STATE_COUNT"         "${LARGE_STATE_COUNT:-0}" "$(baseline_field LARGE_STATE_COUNT)"
    compare "SYMBOL_COUNT"              "${SYMBOL_COUNT:-0}"   "$(baseline_field SYMBOL_COUNT)"
    compare "ALIAS_COUNT"               "${ALIAS_COUNT:-0}"    "$(baseline_field ALIAS_COUNT)"
    compare "TOKEN_COUNT"               "${TOKEN_COUNT:-0}"    "$(baseline_field TOKEN_COUNT)"
else
    echo ""
    echo "no baseline at $BASELINE - run 'measure.sh --capture-baseline' to seed."
fi

echo ""
echo "(skipped tree-sitter test - run manually if needed:"
echo "   cd crates/lang/sysml-parser-incremental/tree-sitter && npx tree-sitter test)"
