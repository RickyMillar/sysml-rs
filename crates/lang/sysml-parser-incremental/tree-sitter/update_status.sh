#!/bin/bash
# update_status.sh — Runs tests and updates TREE_SITTER_STATUS.md
# Usage: ./update_status.sh [--quick]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STATUS_FILE="$SCRIPT_DIR/TREE_SITTER_STATUS.md"
CORPUS_DIR="$SCRIPT_DIR/../../references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests"
EXEC_DIR="$SCRIPT_DIR/test/execution_corpus"
QUICK="${1:-}"

cd "$SCRIPT_DIR"

echo "=== Tree-sitter Status Update ==="
echo ""

# --- CLI info ---
TS_VERSION=$(npx tree-sitter --version 2>&1 | head -1)
echo "CLI: $TS_VERSION"

# --- parser.c size ---
PARSER_SIZE="N/A"
if [ -f src/parser.c ]; then
    PARSER_SIZE=$(du -h src/parser.c | cut -f1)
fi
echo "parser.c: $PARSER_SIZE"

# --- Conflict count ---
CONFLICT_COUNT=$(node -e "const c = require('./helpers/conflicts'); console.log(c({}).length)" 2>/dev/null || echo "?")
echo "Conflicts: $CONFLICT_COUNT"

# --- Internal tests ---
echo ""
echo "--- Internal corpus tests ---"
INTERNAL_RESULT=$(npx tree-sitter test 2>&1 || true)
INTERNAL_PASS=$(echo "$INTERNAL_RESULT" | grep -c "✓" 2>/dev/null || echo 0)
INTERNAL_FAIL=$(echo "$INTERNAL_RESULT" | grep -c "✗" 2>/dev/null || echo 0)
INTERNAL_TOTAL=$((INTERNAL_PASS + INTERNAL_FAIL))
echo "Internal: $INTERNAL_PASS/$INTERNAL_TOTAL"

if [ "$QUICK" = "--quick" ]; then
    echo ""
    echo "Quick mode: skipping library coverage"
    echo ""
    echo "=== Summary ==="
    echo "CLI: $TS_VERSION"
    echo "parser.c: $PARSER_SIZE"
    echo "Conflicts: $CONFLICT_COUNT"
    echo "Internal tests: $INTERNAL_PASS/$INTERNAL_TOTAL"
    exit 0
fi

# --- Library coverage (Tier 1) ---
echo ""
echo "--- Library coverage (Tier 1) ---"
TMPFILE=$(mktemp)
lib_pass=0
lib_fail=0
lib_total=0
lib_failing=""

for dir in "$CORPUS_DIR"/library.kernel "$CORPUS_DIR"/library.systems "$CORPUS_DIR"/library.domain/*; do
    [ -d "$dir" ] || continue
    for f in "$dir"/*.sysml "$dir"/*.kerml; do
        [ -f "$f" ] || continue
        lib_total=$((lib_total + 1))
        rel="${f#$CORPUS_DIR/}"
        npx tree-sitter parse "$f" > "$TMPFILE" 2>/dev/null || true
        if /bin/grep -q "ERROR" "$TMPFILE"; then
            lib_fail=$((lib_fail + 1))
            lib_failing="$lib_failing\n- \`$rel\`"
        else
            lib_pass=$((lib_pass + 1))
        fi
    done
done
rm -f "$TMPFILE"

lib_pct=0
[ $lib_total -gt 0 ] && lib_pct=$((lib_pass * 100 / lib_total))
echo "Library: $lib_pass/$lib_total ($lib_pct%)"

# --- Execution corpus (Tier 4) ---
echo ""
echo "--- Execution corpus (Tier 4) ---"
TMPFILE=$(mktemp)
exec_pass=0
exec_fail=0
exec_total=0
exec_failing=""

if [ -d "$EXEC_DIR" ]; then
    for f in "$EXEC_DIR"/*.sysml; do
        [ -f "$f" ] || continue
        exec_total=$((exec_total + 1))
        rel="$(basename "$f")"
        npx tree-sitter parse "$f" > "$TMPFILE" 2>/dev/null || true
        if /bin/grep -q "ERROR" "$TMPFILE"; then
            exec_fail=$((exec_fail + 1))
            exec_failing="$exec_failing\n- \`$rel\`"
        else
            exec_pass=$((exec_pass + 1))
        fi
    done
fi
rm -f "$TMPFILE"

exec_pct=0
[ $exec_total -gt 0 ] && exec_pct=$((exec_pass * 100 / exec_total))
echo "Execution: $exec_pass/$exec_total ($exec_pct%)"

# --- Summary ---
echo ""
echo "=== Summary ==="
echo "CLI: $TS_VERSION"
echo "parser.c: $PARSER_SIZE"
echo "Conflicts: $CONFLICT_COUNT"
echo "Internal tests: $INTERNAL_PASS/$INTERNAL_TOTAL"
echo "Library: $lib_pass/$lib_total ($lib_pct%)"
echo "Execution: $exec_pass/$exec_total ($exec_pct%)"

if [ -n "$lib_failing" ]; then
    echo ""
    echo "Failing library files:"
    echo -e "$lib_failing"
fi

if [ -n "$exec_failing" ]; then
    echo ""
    echo "Failing execution files:"
    echo -e "$exec_failing"
fi
