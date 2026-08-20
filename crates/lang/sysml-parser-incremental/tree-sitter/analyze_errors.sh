#!/bin/bash
# Analyze tree-sitter parse errors to find patterns
# Usage: ./analyze_errors.sh [--limit N]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
CORPUS_ROOT="${SYSML_CORPUS_PATH:-$REPO_ROOT/references/sysmlv2}"
LIMIT="${1:-20}"

if [[ ! -d "$CORPUS_ROOT" ]]; then
    echo "error: corpus root '$CORPUS_ROOT' does not exist" >&2
    echo "       set SYSML_CORPUS_PATH to a directory containing .sysml files" >&2
    exit 1
fi

cd "$SCRIPT_DIR"

echo "=== Error Pattern Analysis ==="
echo "Analyzing first $LIMIT files with errors..."
echo ""

# Create temp file for collecting error contexts
CONTEXTS=$(mktemp)

# Find files and analyze
find "$CORPUS_ROOT" -name "*.sysml" 2>/dev/null | head -100 | while read FILE; do
    OUTPUT=$(tree-sitter parse "$FILE" 2>&1)

    if echo "$OUTPUT" | grep -qE "ERROR|MISSING"; then
        # Extract file content around errors
        echo "$OUTPUT" | grep -E "(ERROR|MISSING)" | while read ERROR_LINE; do
            # Get row number
            ROW=$(echo "$ERROR_LINE" | grep -oP '\[\K\d+' | head -1)
            if [[ -n "$ROW" ]]; then
                LINENUM=$((ROW + 1))
                CONTENT=$(sed -n "${LINENUM}p" "$FILE" | head -c 80)
                echo "$CONTENT" >> "$CONTEXTS"
            fi
        done
    fi
done

echo "=== Most Common Error Patterns ==="
echo ""

# Analyze patterns
sort "$CONTEXTS" | uniq -c | sort -rn | head -30 | while read COUNT PATTERN; do
    printf "%4d: %s\n" "$COUNT" "$PATTERN"
done

echo ""
echo "=== Keyword Analysis ==="
echo ""

# Check for common keywords that might be missing
for KEYWORD in "standard library" ":>>" "doc" "abstract" "ref" "flow" "message" "metadata"; do
    COUNT=$(grep -c "$KEYWORD" "$CONTEXTS" 2>/dev/null || echo 0)
    if [[ $COUNT -gt 0 ]]; then
        printf "%4d: %s\n" "$COUNT" "$KEYWORD"
    fi
done

rm -f "$CONTEXTS"
