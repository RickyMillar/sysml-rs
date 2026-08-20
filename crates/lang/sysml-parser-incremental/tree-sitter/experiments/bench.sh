#!/bin/bash
# Tree-sitter grammar optimization benchmark harness
#
# Creates an isolated copy of the grammar in /tmp, runs tree-sitter generate
# and tests, then records metrics to experiments/results/<name>.json.
#
# Usage:
#   ./bench.sh <experiment-name>                  # Interactive: edit grammar in copy, then generate
#   ./bench.sh <experiment-name> --no-generate     # Skip generate (use existing parser.c in copy)
#   ./bench.sh baseline                           # Capture current grammar metrics as baseline
#
# The copy at /tmp/ts-exp-<name>/ contains the full grammar source so you can
# edit it before running generate. Use --no-generate if you already ran generate
# manually in the copy.
#
# CRITICAL: Uses --abi=14 (Rust crate is tree-sitter 0.22.6, ABI 13-14 only).
# ABI 15 causes SIGSEGV. Do NOT change this.
#
# Environment:
#   SYSML_CORPUS_PATH  - Path to SysML v2 references (auto-detected from repo structure)
#   SKIP_LIBRARY       - Set to 1 to skip library coverage test (saves ~5 min)
#   SKIP_CORPUS        - Set to 1 to skip corpus test
#
# Output: experiments/results/<name>.json with metrics

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results"

# --- Argument parsing ---
NAME=""
NO_GENERATE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-generate) NO_GENERATE=true; shift ;;
        --help|-h)
            head -20 "$0" | tail -18
            exit 0
            ;;
        *)
            if [[ -z "$NAME" ]]; then
                NAME="$1"
            else
                echo "ERROR: unexpected argument: $1" >&2
                exit 1
            fi
            shift
            ;;
    esac
done

if [[ -z "$NAME" ]]; then
    echo "Usage: ./bench.sh <experiment-name> [--no-generate]" >&2
    echo "  e.g. ./bench.sh baseline" >&2
    echo "  e.g. ./bench.sh conflict-prune" >&2
    exit 1
fi

# --- Detect corpus path ---
if [[ -z "${SYSML_CORPUS_PATH:-}" ]]; then
    SYSML_CORPUS_PATH="$TS_DIR/../../references/sysmlv2"
fi
if [[ ! -d "$SYSML_CORPUS_PATH" ]]; then
    echo "WARNING: SYSML_CORPUS_PATH=$SYSML_CORPUS_PATH not found, library test will be skipped" >&2
    SKIP_LIBRARY=1
fi
export SYSML_CORPUS_PATH

# --- Setup experiment copy ---
EXP_DIR="/tmp/ts-exp-${NAME}"

if [[ "$NAME" == "baseline" ]]; then
    # Baseline: copy current grammar as-is
    echo "=== Capturing baseline from current grammar ==="
fi

if [[ -d "$EXP_DIR" && "$NO_GENERATE" == "true" ]]; then
    echo "Reusing existing experiment copy at $EXP_DIR"
else
    echo "Creating experiment copy at $EXP_DIR ..."
    rm -rf "$EXP_DIR"
    mkdir -p "$EXP_DIR"

    # Copy only the files needed for generate + test
    # grammar.js + rules/ + helpers/ + generated/ = the grammar source
    cp "$TS_DIR/grammar.js" "$EXP_DIR/"
    cp -r "$TS_DIR/rules" "$EXP_DIR/"
    cp -r "$TS_DIR/helpers" "$EXP_DIR/"
    cp -r "$TS_DIR/generated" "$EXP_DIR/"

    # Package files for npm ci
    cp "$TS_DIR/package.json" "$EXP_DIR/"
    cp "$TS_DIR/package-lock.json" "$EXP_DIR/"
    cp "$TS_DIR/tree-sitter.json" "$EXP_DIR/"

    # Test corpus (for tree-sitter test)
    cp -r "$TS_DIR/test" "$EXP_DIR/"

    # Query files (tree-sitter test may need them)
    cp -r "$TS_DIR/queries" "$EXP_DIR/"

    # Copy src/ if it exists (for --no-generate reuse)
    if [[ -d "$TS_DIR/src" ]]; then
        cp -r "$TS_DIR/src" "$EXP_DIR/"
    fi

    # Copy library test script
    if [[ -d "$SYSML_CORPUS_PATH" ]]; then
        cp "$TS_DIR/test_library.sh" "$EXP_DIR/"
    fi

    # Install dependencies
    # --ignore-scripts avoids node-gyp-build failure (no native bindings in copy)
    # Then manually run tree-sitter-cli install to get the binary
    echo "Running npm ci ..."
    (cd "$EXP_DIR" && npm ci --ignore-scripts 2>&1 | tail -3)
    # Ensure tree-sitter binary exists (symlink from main install if needed)
    if [[ ! -f "$EXP_DIR/node_modules/tree-sitter-cli/tree-sitter" ]]; then
        if [[ -f "$TS_DIR/node_modules/tree-sitter-cli/tree-sitter" ]]; then
            ln -sf "$TS_DIR/node_modules/tree-sitter-cli/tree-sitter" \
                "$EXP_DIR/node_modules/tree-sitter-cli/tree-sitter"
        else
            echo "WARNING: tree-sitter binary not found, tests will fail" >&2
        fi
    fi
    echo "npm ci done."
fi

echo ""
echo "=== Experiment: $NAME ==="
echo "Copy location: $EXP_DIR"
echo ""

# --- Generate ---
GENERATE_TIME_SEC=0
GENERATE_EXIT=0

if [[ "$NO_GENERATE" == "false" ]]; then
    echo "Running tree-sitter generate --abi=14 ..."
    echo "(This typically takes 45-60 minutes)"
    echo ""

    GEN_START=$(date +%s)
    set +e
    (cd "$EXP_DIR" && npx tree-sitter generate --abi=14 2>&1) | tee "/tmp/ts-exp-${NAME}-generate.log"
    GENERATE_EXIT=${PIPESTATUS[0]}
    set -e
    GEN_END=$(date +%s)
    GENERATE_TIME_SEC=$((GEN_END - GEN_START))

    GEN_MIN=$((GENERATE_TIME_SEC / 60))
    GEN_SEC=$((GENERATE_TIME_SEC % 60))
    echo ""
    echo "Generate completed in ${GEN_MIN}m ${GEN_SEC}s (exit code: $GENERATE_EXIT)"
else
    echo "Skipping generate (--no-generate)"
fi

if [[ $GENERATE_EXIT -ne 0 ]]; then
    echo "ERROR: tree-sitter generate failed! See /tmp/ts-exp-${NAME}-generate.log" >&2
    # Still write a result so we know it failed
    mkdir -p "$RESULTS_DIR"
    cat > "$RESULTS_DIR/${NAME}.json" <<ENDJSON
{
  "name": "$NAME",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "generate_time_sec": $GENERATE_TIME_SEC,
  "parser_c_bytes": 0,
  "state_count": 0,
  "large_state_count": 0,
  "corpus_pass": 0,
  "corpus_total": 0,
  "library_pass": 0,
  "library_total": 0,
  "exit_code": $GENERATE_EXIT,
  "error": "generate failed"
}
ENDJSON
    echo "Result written to experiments/results/${NAME}.json"
    exit 1
fi

# --- Extract metrics from parser.c ---
PARSER_C="$EXP_DIR/src/parser.c"
PARSER_C_BYTES=0
STATE_COUNT=0
LARGE_STATE_COUNT=0

if [[ -f "$PARSER_C" ]]; then
    PARSER_C_BYTES=$(wc -c < "$PARSER_C")
    STATE_COUNT=$(grep '#define STATE_COUNT' "$PARSER_C" | awk '{print $3}' || echo 0)
    LARGE_STATE_COUNT=$(grep '#define LARGE_STATE_COUNT' "$PARSER_C" | awk '{print $3}' || echo 0)
    PARSER_MB=$(echo "scale=1; $PARSER_C_BYTES / 1048576" | bc)
    echo ""
    echo "Parser metrics:"
    echo "  parser.c size: ${PARSER_MB} MB ($PARSER_C_BYTES bytes)"
    echo "  STATE_COUNT: $STATE_COUNT"
    echo "  LARGE_STATE_COUNT: $LARGE_STATE_COUNT"
fi

# --- Run corpus test (tree-sitter test) ---
CORPUS_PASS=0
CORPUS_TOTAL=0

if [[ "${SKIP_CORPUS:-0}" != "1" ]]; then
    echo ""
    echo "Running tree-sitter test ..."
    set +e
    TEST_OUTPUT=$(cd "$EXP_DIR" && npx tree-sitter test 2>&1)
    TEST_EXIT=$?
    set -e

    # Output format: "Total parses: 126; successful parses: 126; failed parses: 0; ..."
    if echo "$TEST_OUTPUT" | grep -q "Total parses:"; then
        CORPUS_TOTAL=$(echo "$TEST_OUTPUT" | grep -oP 'Total parses: \K[0-9]+')
        CORPUS_PASS=$(echo "$TEST_OUTPUT" | grep -oP 'successful parses: \K[0-9]+')
    else
        # Fallback: count checkmark/cross lines
        CORPUS_PASS=$(echo "$TEST_OUTPUT" | grep -c '✓' || true)
        CORPUS_FAIL=$(echo "$TEST_OUTPUT" | grep -c '✗' || true)
        CORPUS_TOTAL=$((CORPUS_PASS + CORPUS_FAIL))
    fi
    # Ensure numeric (default 0 if empty)
    CORPUS_PASS=${CORPUS_PASS:-0}
    CORPUS_TOTAL=${CORPUS_TOTAL:-0}
    echo "Corpus: $CORPUS_PASS/$CORPUS_TOTAL"
else
    echo "Skipping corpus test (SKIP_CORPUS=1)"
fi

# --- Run library test ---
LIBRARY_PASS=0
LIBRARY_TOTAL=0

if [[ "${SKIP_LIBRARY:-0}" != "1" && -f "$EXP_DIR/test_library.sh" ]]; then
    echo ""
    echo "Running library coverage test ..."

    # Patch test_library.sh to use actual corpus path
    # The script looks for corpus relative to its own location, but our copy is in /tmp
    set +e
    LIB_OUTPUT=$(cd "$EXP_DIR" && SYSML_CORPUS_PATH="$SYSML_CORPUS_PATH" \
        bash -c 'CORPUS_DIR="'"$SYSML_CORPUS_PATH"'/SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests"
        TMPFILE=$(mktemp)
        pass=0; fail=0; total=0
        for dir in "$CORPUS_DIR"/library.kernel "$CORPUS_DIR"/library.systems "$CORPUS_DIR"/library.domain/*; do
            [ -d "$dir" ] || continue
            for f in "$dir"/*.sysml "$dir"/*.kerml; do
                [ -f "$f" ] || continue
                total=$((total + 1))
                npx tree-sitter parse "$f" > "$TMPFILE" 2>/dev/null || true
                if /bin/grep -q "ERROR" "$TMPFILE"; then
                    fail=$((fail + 1))
                else
                    pass=$((pass + 1))
                fi
            done
        done
        rm -f "$TMPFILE"
        echo "$pass/$total"' 2>/dev/null)
    set -e

    if [[ "$LIB_OUTPUT" =~ ([0-9]+)/([0-9]+) ]]; then
        LIBRARY_PASS="${BASH_REMATCH[1]}"
        LIBRARY_TOTAL="${BASH_REMATCH[2]}"
    fi
    echo "Library: $LIBRARY_PASS/$LIBRARY_TOTAL"
else
    echo "Skipping library test (SKIP_LIBRARY=1 or test_library.sh not found)"
fi

# --- Write results JSON ---
mkdir -p "$RESULTS_DIR"
RESULT_FILE="$RESULTS_DIR/${NAME}.json"

cat > "$RESULT_FILE" <<ENDJSON
{
  "name": "$NAME",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "generate_time_sec": $GENERATE_TIME_SEC,
  "parser_c_bytes": $PARSER_C_BYTES,
  "state_count": $STATE_COUNT,
  "large_state_count": $LARGE_STATE_COUNT,
  "corpus_pass": $CORPUS_PASS,
  "corpus_total": $CORPUS_TOTAL,
  "library_pass": $LIBRARY_PASS,
  "library_total": $LIBRARY_TOTAL,
  "exit_code": $GENERATE_EXIT
}
ENDJSON

echo ""
echo "=== Results written to experiments/results/${NAME}.json ==="
cat "$RESULT_FILE"
echo ""

# --- Compare with baseline if it exists ---
BASELINE_FILE="$RESULTS_DIR/baseline.json"
if [[ -f "$BASELINE_FILE" && "$NAME" != "baseline" ]]; then
    BASE_SIZE=$(python3 -c "import json; print(json.load(open('$BASELINE_FILE'))['parser_c_bytes'])" 2>/dev/null || echo 0)
    if [[ "$BASE_SIZE" -gt 0 ]]; then
        DELTA=$(echo "scale=1; ($PARSER_C_BYTES - $BASE_SIZE) * 100 / $BASE_SIZE" | bc)
        echo "Delta vs baseline: ${DELTA}% parser.c size"
    fi
fi

echo ""
echo "Experiment copy preserved at: $EXP_DIR"
echo "To re-run without generate: ./bench.sh $NAME --no-generate"
