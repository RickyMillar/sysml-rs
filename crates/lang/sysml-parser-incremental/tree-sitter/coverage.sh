#!/bin/bash
# Compare tree-sitter grammar coverage against xtext specification
# Usage: ./coverage.sh [--detailed] [--json]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
REFS_DIR="${SYSML_REFS_DIR:-$REPO_ROOT/references/sysmlv2}"
GRAMMAR_FILE="$SCRIPT_DIR/grammar.js"

SYSML_XTEXT="$REFS_DIR/SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext"

if [[ ! -f "$SYSML_XTEXT" ]]; then
    echo "error: SysML.xtext not found under '$REFS_DIR'" >&2
    echo "       expected: $SYSML_XTEXT" >&2
    echo "       set SYSML_REFS_DIR to a checkout that contains SysML-v2-Pilot-Implementation/" >&2
    exit 1
fi

DETAILED=false
JSON=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --detailed|-d)
            DETAILED=true
            shift
            ;;
        --json|-j)
            JSON=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Compare tree-sitter grammar coverage against xtext specification"
            echo ""
            echo "Options:"
            echo "  --detailed, -d  Show detailed missing rules by category"
            echo "  --json, -j      Output as JSON"
            echo "  --help, -h      Show this help"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Extract xtext rule names (rules that return SysML types)
extract_xtext_rules() {
    grep -oP "^[A-Z][a-zA-Z]+(?= returns)" "$SYSML_XTEXT" | sort -u
}

# Extract tree-sitter rule names
extract_treesitter_rules() {
    grep -oP '^\s+_?[a-z_]+(?=:\s*\(\$\)\s*=>)' "$GRAMMAR_FILE" | sed 's/^\s*//' | sort -u
}

# Convert xtext rule name to tree-sitter style (PascalCase -> snake_case)
to_snake_case() {
    echo "$1" | sed 's/\([A-Z]\)/_\L\1/g' | sed 's/^_//'
}

# Categorize rules
categorize_rule() {
    local rule="$1"
    case "$rule" in
        *Definition) echo "definitions" ;;
        *Usage) echo "usages" ;;
        *Member) echo "membership" ;;
        *Expression|*Operator) echo "expressions" ;;
        *Typing|*Subsetting|*Redefinition) echo "specialization" ;;
        *Annotation|*Comment|*Documentation|*Metadata) echo "annotations" ;;
        *Transition|*State*|*Action*) echo "behavior" ;;
        *Requirement*|*Constraint*|*Concern*) echo "requirements" ;;
        *Port*|*Interface*|*Connection*|*Flow*) echo "connections" ;;
        Package|Namespace|Import|*Import*|*Expose*) echo "packages" ;;
        *) echo "other" ;;
    esac
}

# Get xtext rules
XTEXT_RULES=$(extract_xtext_rules)
XTEXT_COUNT=$(echo "$XTEXT_RULES" | wc -l)

# Get tree-sitter rules
TS_RULES=$(extract_treesitter_rules)
TS_COUNT=$(echo "$TS_RULES" | wc -l)

# Find covered rules (xtext rules that have a tree-sitter equivalent)
COVERED=0
MISSING_RULES=""

while IFS= read -r rule; do
    snake=$(to_snake_case "$rule")
    if echo "$TS_RULES" | grep -qw "$snake"; then
        COVERED=$((COVERED + 1))
    else
        MISSING_RULES="$MISSING_RULES$rule"$'\n'
    fi
done <<< "$XTEXT_RULES"

MISSING_COUNT=$((XTEXT_COUNT - COVERED))
COVERAGE_PCT=$((COVERED * 100 / XTEXT_COUNT))

if $JSON; then
    echo "{"
    echo "  \"xtext_rules\": $XTEXT_COUNT,"
    echo "  \"treesitter_rules\": $TS_COUNT,"
    echo "  \"covered\": $COVERED,"
    echo "  \"missing\": $MISSING_COUNT,"
    echo "  \"coverage_percent\": $COVERAGE_PCT"
    echo "}"
    exit 0
fi

echo "=== Tree-sitter Grammar Coverage ==="
echo ""
echo "Xtext rules:       $XTEXT_COUNT"
echo "Tree-sitter rules: $TS_COUNT"
echo "Covered:           $COVERED"
echo "Missing:           $MISSING_COUNT"
echo "Coverage:          ${COVERAGE_PCT}%"

if $DETAILED && [[ -n "$MISSING_RULES" ]]; then
    echo ""
    echo "=== Missing Rules by Category ==="

    # Categorize missing rules
    declare -A CATEGORIES
    while IFS= read -r rule; do
        [[ -z "$rule" ]] && continue
        cat=$(categorize_rule "$rule")
        CATEGORIES[$cat]="${CATEGORIES[$cat]}  - $rule"$'\n'
    done <<< "$MISSING_RULES"

    # Print by category
    for cat in definitions usages behavior connections requirements expressions specialization membership packages annotations other; do
        if [[ -n "${CATEGORIES[$cat]}" ]]; then
            count=$(echo -n "${CATEGORIES[$cat]}" | grep -c "^  -" || echo 0)
            echo ""
            echo "$cat ($count missing):"
            echo -n "${CATEGORIES[$cat]}"
        fi
    done
fi

echo ""
echo "=== Quick Reference ==="
echo "Run './coverage.sh --detailed' for categorized missing rules"
echo "Run './test_corpus.sh --quick' to see current parse success rate"
