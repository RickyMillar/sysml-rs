#!/bin/bash
# Diagnose tree-sitter parse errors for a specific file
# Usage: ./diagnose.sh <file>

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
cd "$SCRIPT_DIR"

FILE="$1"

if [[ -z "$FILE" ]]; then
    echo "Usage: $0 <file.sysml>"
    echo ""
    echo "Examples:"
    echo "  $0 /path/to/file.sysml"
    echo "  $0 Actions.sysml   # Searches corpus for matching file"
    exit 1
fi

# If file doesn't exist, try to find it in corpus
if [[ ! -f "$FILE" ]]; then
    CORPUS="${SYSML_CORPUS_PATH:-$REPO_ROOT/references/sysmlv2}"
    if [[ ! -d "$CORPUS" ]]; then
        echo "error: corpus root '$CORPUS' does not exist" >&2
        echo "       set SYSML_CORPUS_PATH, or pass a path to an existing file" >&2
        exit 1
    fi
    FOUND=$(find "$CORPUS" -name "$FILE" 2>/dev/null | head -1)
    if [[ -n "$FOUND" ]]; then
        FILE="$FOUND"
        echo "Found: $FILE"
        echo ""
    else
        echo "File not found: $FILE"
        exit 1
    fi
fi

echo "=== File: $(basename "$FILE") ==="
echo ""

# Parse and show full tree
echo "=== Parse Tree ==="
OUTPUT=$(tree-sitter parse "$FILE" 2>&1)
echo "$OUTPUT"

echo ""
echo "=== Error Summary ==="

# Count errors
ERROR_COUNT=$(echo "$OUTPUT" | grep -c "ERROR" || echo 0)
MISSING_COUNT=$(echo "$OUTPUT" | grep -c "MISSING" || echo 0)

echo "ERROR nodes: $ERROR_COUNT"
echo "MISSING nodes: $MISSING_COUNT"

if [[ $ERROR_COUNT -eq 0 && $MISSING_COUNT -eq 0 ]]; then
    echo ""
    echo "✓ No parse errors!"
    exit 0
fi

echo ""
echo "=== Error Locations ==="
# Extract error locations with context
echo "$OUTPUT" | grep -E "(ERROR|MISSING)" | head -20

echo ""
echo "=== Source Context ==="
# Show lines with errors
echo "$OUTPUT" | grep -E "(ERROR|MISSING)" | while read LINE; do
    # Extract line number from [row, col]
    ROW=$(echo "$LINE" | grep -oP '\[\K\d+' | head -1)
    if [[ -n "$ROW" ]]; then
        LINENUM=$((ROW + 1))
        echo "Line $LINENUM:"
        sed -n "${LINENUM}p" "$FILE"
        echo ""
    fi
done | head -40

exit 1
