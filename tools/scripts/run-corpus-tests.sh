#!/usr/bin/env bash
set -euo pipefail

# Run all corpus validation tests against the reference specification.
# Usage: ./tools/scripts/run-corpus-tests.sh [extra cargo test args...]
#
# Examples:
#   ./tools/scripts/run-corpus-tests.sh                    # Run all corpus tests
#   ./tools/scripts/run-corpus-tests.sh -- advent           # Run only advent tests
#   ./tools/scripts/run-corpus-tests.sh --nocapture         # Show test output

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

export SYSML_CORPUS_PATH="${SYSML_CORPUS_PATH:-$ROOT_DIR/references/sysmlv2}"

echo "Corpus path: $SYSML_CORPUS_PATH"
echo "Running corpus tests..."
echo ""

cd "$ROOT_DIR"
cargo test -p sysml-spec-tests -- --ignored "$@"
