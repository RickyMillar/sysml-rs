#!/bin/bash
# Resolution benchmark script with optional flamegraph generation
#
# Usage:
#   ./scripts/bench.sh              # Quick timing test
#   ./scripts/bench.sh --flame      # Generate flamegraph
#   ./scripts/bench.sh --compare    # Compare with baseline
#   ./scripts/bench.sh --save NAME  # Save baseline with name

set -e
cd "$(dirname "$0")/.."

export SYSML_CORPUS_PATH="${SYSML_CORPUS_PATH:-$(pwd)/references/sysmlv2}"
BASELINE_DIR="benchmarks/baselines"
FLAME_DIR="benchmarks/flamegraphs"

mkdir -p "$BASELINE_DIR" "$FLAME_DIR"

run_benchmark() {
    cargo run --release -p sysml-core --example bench_resolution 2>/dev/null
}

case "${1:-}" in
    --flame)
        NAME="${2:-$(date +%Y%m%d_%H%M%S)}"
        OUTPUT="$FLAME_DIR/resolution_$NAME.svg"
        echo "Generating flamegraph: $OUTPUT"
        cargo flamegraph -p sysml-core --example bench_resolution -o "$OUTPUT" 2>/dev/null
        echo "Flamegraph saved to: $OUTPUT"
        ;;

    --save)
        NAME="${2:-baseline}"
        OUTPUT="$BASELINE_DIR/$NAME.txt"
        echo "Saving baseline: $OUTPUT"
        run_benchmark | tee "$OUTPUT"
        echo "Baseline saved to: $OUTPUT"
        ;;

    --compare)
        BASELINE="${2:-$BASELINE_DIR/baseline.txt}"
        if [ ! -f "$BASELINE" ]; then
            echo "Error: Baseline not found: $BASELINE"
            echo "Run: ./scripts/bench.sh --save baseline"
            exit 1
        fi

        echo "=== Current ==="
        CURRENT=$(run_benchmark)
        echo "$CURRENT"

        echo ""
        echo "=== Baseline ($BASELINE) ==="
        cat "$BASELINE"

        echo ""
        echo "=== Comparison ==="
        CURR_MS=$(echo "$CURRENT" | grep BENCHMARK_RESOLVE_AVG_MS | cut -d= -f2)
        BASE_MS=$(grep BENCHMARK_RESOLVE_AVG_MS "$BASELINE" | cut -d= -f2)

        if [ -n "$CURR_MS" ] && [ -n "$BASE_MS" ]; then
            DIFF=$((CURR_MS - BASE_MS))
            PCT=$(echo "scale=1; ($DIFF * 100) / $BASE_MS" | bc)
            if [ "$DIFF" -lt 0 ]; then
                echo "Resolution: ${CURR_MS}ms vs ${BASE_MS}ms (${PCT}% faster)"
            else
                echo "Resolution: ${CURR_MS}ms vs ${BASE_MS}ms (+${PCT}% slower)"
            fi
        fi
        ;;

    --help|-h)
        echo "Usage: $0 [--flame [NAME]] [--save NAME] [--compare [BASELINE]]"
        echo ""
        echo "Options:"
        echo "  (none)           Run quick benchmark with timing"
        echo "  --flame [NAME]   Generate flamegraph (saved to benchmarks/flamegraphs/)"
        echo "  --save NAME      Save current results as baseline"
        echo "  --compare [FILE] Compare current results with baseline"
        echo ""
        echo "Environment:"
        echo "  SYSML_CORPUS_PATH  Path to sysmlv2 references (default: ./references/sysmlv2)"
        echo "  BENCH_ITERATIONS   Number of resolution iterations (default: 5)"
        ;;

    *)
        run_benchmark
        ;;
esac
