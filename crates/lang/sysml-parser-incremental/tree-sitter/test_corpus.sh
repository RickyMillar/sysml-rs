#!/bin/bash
# Test tree-sitter grammar against the SysML v2 corpus
# Usage: ./test_corpus.sh [--quick] [--verbose] [--pattern GLOB]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
CORPUS_ROOT="${SYSML_CORPUS_PATH:-$REPO_ROOT/references/sysmlv2}"

if [[ ! -d "$CORPUS_ROOT" ]]; then
    echo "error: corpus root '$CORPUS_ROOT' does not exist" >&2
    echo "       set SYSML_CORPUS_PATH to a directory containing .sysml files" >&2
    exit 1
fi

QUICK=false
VERBOSE=false
PATTERN=""
LIMIT=0

while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)
            QUICK=true
            LIMIT=30
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --pattern)
            PATTERN="$2"
            shift 2
            ;;
        --limit)
            LIMIT="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --quick       Test only 30 files (faster iteration)"
            echo "  --verbose     Show each file as it's tested"
            echo "  --pattern X   Filter files matching pattern"
            echo "  --limit N     Limit to N files"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Change to grammar directory so tree-sitter finds the grammar
cd "$SCRIPT_DIR"

echo "=== Tree-sitter SysML Corpus Test ==="
echo "Corpus: $CORPUS_ROOT"

# Build file list
if [[ -n "$PATTERN" ]]; then
    FILES=$(find "$CORPUS_ROOT" -name "*.sysml" -path "*$PATTERN*" 2>/dev/null | sort)
elif $QUICK; then
    FILES=$(find "$CORPUS_ROOT" -name "*.sysml" 2>/dev/null | sort | head -$LIMIT)
else
    FILES=$(find "$CORPUS_ROOT" -name "*.sysml" 2>/dev/null | sort)
fi

# Apply limit if set
if [[ $LIMIT -gt 0 ]] && ! $QUICK; then
    FILES=$(echo "$FILES" | head -$LIMIT)
fi

TOTAL=$(echo "$FILES" | grep -c . || echo 0)
echo "Files: $TOTAL"
echo ""

if [[ $TOTAL -eq 0 ]]; then
    echo "No .sysml files found"
    exit 1
fi

PASSED=0
FAILED=0
ERRORS_FILE=$(mktemp)

echo "$FILES" | while read FILE; do
    [[ -z "$FILE" ]] && continue

    RELATIVE="${FILE#$CORPUS_ROOT/}"

    # Parse with tree-sitter and check for ERROR nodes
    OUTPUT=$(tree-sitter parse "$FILE" 2>&1)

    if echo "$OUTPUT" | grep -qE "ERROR|MISSING"; then
        echo "$RELATIVE" >> "$ERRORS_FILE"
        if $VERBOSE; then
            echo "✗ $RELATIVE"
        fi
    else
        if $VERBOSE; then
            echo "✓ $RELATIVE"
        fi
    fi
done

# Count results
FAILED=$(wc -l < "$ERRORS_FILE")
PASSED=$((TOTAL - FAILED))

echo ""
echo "=== Results ==="
echo "Passed: $PASSED / $TOTAL ($(( PASSED * 100 / TOTAL ))%)"
echo "Failed: $FAILED / $TOTAL"

if [[ $FAILED -gt 0 ]]; then
    echo ""
    echo "=== Failed Files (first 20) ==="
    head -20 "$ERRORS_FILE" | while read F; do
        echo "  $F"
    done
    if [[ $FAILED -gt 20 ]]; then
        echo "  ... and $((FAILED - 20)) more"
    fi
fi

rm -f "$ERRORS_FILE"

# Exit with error if any failures
[[ $FAILED -gt 0 ]] && exit 1
exit 0
