#!/bin/bash
# Generates SModel JSON fixtures from vis-coverage test files
# Validates parse status before generation
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURES_DIR="$ROOT_DIR/editors/diagram/fixtures"
SYSML_DIR="$ROOT_DIR/tests/vis-coverage"

echo "=== SModel Fixture Generator ==="
echo "Source: $SYSML_DIR"
echo "Output: $FIXTURES_DIR"
echo ""

total=0
generated=0
skipped=0

for file in "$SYSML_DIR"/*.sysml; do
    [ -f "$file" ] || continue
    name=$(basename "$file" .sysml)
    total=$((total + 1))

    # Pre-flight: check for parse errors
    errors=$(cargo run -q -p sysml-cli -- inspect "$file" --diagnostics --json --no-stdlib 2>/dev/null \
        | python3 -c "import sys,json; d=json.load(sys.stdin); print(sum(1 for x in d if x['severity']=='error'))" 2>/dev/null || echo "1")

    if [ "$errors" -gt 0 ]; then
        echo "  SKIP $name ($errors parse errors)"
        skipped=$((skipped + 1))
        continue
    fi

    # Generate general view (collapsed — spec default: text compartments, not nested boxes)
    echo "  GEN  $name (general)"
    cargo run -q -p sysml-cli -- export smodel "$file" --view general \
        > "$FIXTURES_DIR/test-${name}.json" 2>/dev/null || echo "  WARN: general failed for $name"
    generated=$((generated + 1))

    # Generate expanded variant for testing expand/collapse workflow
    echo "  GEN  $name (general-expanded)"
    cargo run -q -p sysml-cli -- export smodel "$file" --view general --expand-all \
        > "$FIXTURES_DIR/test-${name}-expanded.json" 2>/dev/null || true

    # Generate view-specific fixtures based on file name hints
    case "$name" in
        *ibd*|*interconnection*|*multiport*)
            echo "  GEN  $name (interconnection)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view interconnection \
                > "$FIXTURES_DIR/test-${name}-ibd.json" 2>/dev/null || true
            ;;
        *state*|*control-nodes*)
            echo "  GEN  $name (state)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view state \
                > "$FIXTURES_DIR/test-${name}-state.json" 2>/dev/null || true
            ;;
        *action*|*control-flow*)
            echo "  GEN  $name (action)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view action \
                > "$FIXTURES_DIR/test-${name}-action.json" 2>/dev/null || true
            ;;
        *req*|*traceability*)
            echo "  GEN  $name (requirements)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view requirements \
                > "$FIXTURES_DIR/test-${name}-req.json" 2>/dev/null || true
            echo "  GEN  $name (grid)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view grid \
                > "$FIXTURES_DIR/test-${name}-grid.json" 2>/dev/null || true
            ;;
        *browser*|*nesting*)
            echo "  GEN  $name (browser-collapsed)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view browser \
                > "$FIXTURES_DIR/test-${name}-browser.json" 2>/dev/null || true
            echo "  GEN  $name (browser-expanded)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view browser --expand-all \
                > "$FIXTURES_DIR/test-${name}-browser-expanded.json" 2>/dev/null || true
            ;;
        *sequence*)
            echo "  GEN  $name (sequence)"
            cargo run -q -p sysml-cli -- export smodel "$file" --view sequence \
                > "$FIXTURES_DIR/test-${name}-seq.json" 2>/dev/null || true
            ;;
    esac
done

echo ""
echo "=== Summary ==="
echo "  Total .sysml files: $total"
echo "  Generated: $generated"
echo "  Skipped (parse errors): $skipped"
echo "  Fixtures in: $FIXTURES_DIR/test-*.json"
ls -1 "$FIXTURES_DIR"/test-*.json 2>/dev/null | wc -l | xargs -I{} echo "  Total fixture files: {}"
