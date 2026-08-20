#!/bin/bash
# Tree-sitter library and execution corpus coverage test
# Usage: ./test_library.sh [--fail-only] [--execution] [--tier N] [--all]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CORPUS_DIR="$SCRIPT_DIR/../../../../references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests"
EXEC_DIR="$SCRIPT_DIR/test/execution_corpus"
TMPFILE=$(mktemp)

# Must run tree-sitter from the grammar directory
cd "$SCRIPT_DIR"

# Parse arguments
FAIL_ONLY=false
TIER="1"
SHOW_ALL=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --fail-only) FAIL_ONLY=true; shift ;;
        --execution) TIER="4"; shift ;;
        --tier) TIER="$2"; shift 2 ;;
        --all) SHOW_ALL=true; shift ;;
        *) shift ;;
    esac
done

run_tier() {
    local tier_name="$1"
    local pass=0
    local fail=0
    local total=0

    shift
    for f in "$@"; do
        [ -f "$f" ] || continue
        total=$((total + 1))
        rel="$(basename "$f")"
        npx tree-sitter parse "$f" > "$TMPFILE" 2>/dev/null || true
        if /bin/grep -q "ERROR" "$TMPFILE"; then
            fail=$((fail + 1))
            printf "FAIL  %s\n" "$rel"
        else
            pass=$((pass + 1))
            $FAIL_ONLY || printf "OK    %s\n" "$rel"
        fi
    done

    printf "\n%s: %d/%d (%d%%)\n" "$tier_name" "$pass" "$total" "$(( total > 0 ? pass * 100 / total : 0 ))"
    printf "  PASS: %d  FAIL: %d\n" "$pass" "$fail"
}

# Tier 1: Standard library (kernel + systems + domain)
if [ "$TIER" = "1" ] || [ "$SHOW_ALL" = "true" ]; then
    echo "=== Tier 1: Standard Library ==="
    LIB_FILES=()
    for dir in "$CORPUS_DIR"/library.kernel "$CORPUS_DIR"/library.systems "$CORPUS_DIR"/library.domain/*; do
        [ -d "$dir" ] || continue
        for f in "$dir"/*.sysml "$dir"/*.kerml; do
            [ -f "$f" ] && LIB_FILES+=("$f")
        done
    done
    run_tier "Tier 1 (Library)" "${LIB_FILES[@]}"
    echo ""
fi

# Tier 2: KerML examples
if [ "$TIER" = "2" ] || [ "$SHOW_ALL" = "true" ]; then
    echo "=== Tier 2: KerML Examples ==="
    KERML_FILES=()
    if [ -d "$CORPUS_DIR" ]; then
        while IFS= read -r f; do
            KERML_FILES+=("$f")
        done < <(find "$CORPUS_DIR" -name "*.kerml" -not -path "*/library.*" 2>/dev/null | sort)
    fi
    if [ ${#KERML_FILES[@]} -gt 0 ]; then
        run_tier "Tier 2 (KerML)" "${KERML_FILES[@]}"
    else
        echo "  No KerML example files found"
    fi
    echo ""
fi

# Tier 3: SysML examples
if [ "$TIER" = "3" ] || [ "$SHOW_ALL" = "true" ]; then
    echo "=== Tier 3: SysML Examples ==="
    SYSML_FILES=()
    if [ -d "$CORPUS_DIR" ]; then
        while IFS= read -r f; do
            SYSML_FILES+=("$f")
        done < <(find "$CORPUS_DIR" -name "*.sysml" -not -path "*/library.*" 2>/dev/null | sort)
    fi
    if [ ${#SYSML_FILES[@]} -gt 0 ]; then
        run_tier "Tier 3 (SysML)" "${SYSML_FILES[@]}"
    else
        echo "  No SysML example files found"
    fi
    echo ""
fi

# Tier 4: Execution corpus
if [ "$TIER" = "4" ] || [ "$SHOW_ALL" = "true" ]; then
    echo "=== Tier 4: Execution Corpus ==="
    EXEC_FILES=()
    if [ -d "$EXEC_DIR" ]; then
        for f in "$EXEC_DIR"/*.sysml; do
            [ -f "$f" ] && EXEC_FILES+=("$f")
        done
    fi
    if [ ${#EXEC_FILES[@]} -gt 0 ]; then
        run_tier "Tier 4 (Execution)" "${EXEC_FILES[@]}"
    else
        echo "  No execution corpus files found"
    fi
    echo ""
fi

rm -f "$TMPFILE"
