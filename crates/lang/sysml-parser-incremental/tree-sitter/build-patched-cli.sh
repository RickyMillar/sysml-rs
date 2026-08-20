#!/usr/bin/env bash
# Build a patched tree-sitter CLI with SMALL_STATE_THRESHOLD=200.
#
# Why: tree-sitter hardcodes SMALL_STATE_THRESHOLD=64 in render.rs.
# States with >64 non-empty entries use a dense [STATE][SYMBOL] 2D array;
# smaller states use a compact sparse format. Our grammar has 407 symbols,
# so states with 65-200 entries waste ~60% of their dense array on empty slots.
#
# Patching to 200: LARGE_STATE_COUNT drops from 8,769 to 32.
# Result: parser.c shrinks from 54 MB to 34 MB (-37%), WASM 7.9→3.4 MB (-57%).
# Parse behavior is identical.
#
# Usage:
#   ./build-patched-cli.sh          # Build to default location
#   ./build-patched-cli.sh /path    # Build to custom location
#
# After building, generate with:
#   /tmp/ts-cli-patch/target/release/tree-sitter generate --abi=14

set -euo pipefail

THRESHOLD=${TS_SMALL_STATE_THRESHOLD:-200}
BUILD_DIR="${1:-/tmp/ts-cli-patch}"
CARGO_REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry/src"

echo "=== Building patched tree-sitter CLI (SMALL_STATE_THRESHOLD=$THRESHOLD) ==="

# Find the tree-sitter-generate source in cargo registry
GENERATE_SRC=$(find "$CARGO_REGISTRY" -maxdepth 2 -name "tree-sitter-generate-0.26.5" -type d | head -1)
if [ -z "$GENERATE_SRC" ]; then
    echo "Error: tree-sitter-generate-0.26.5 not found in cargo registry."
    echo "Run 'cargo install tree-sitter-cli@0.26.5' first to populate the cache."
    exit 1
fi

CLI_SRC=$(find "$CARGO_REGISTRY" -maxdepth 2 -name "tree-sitter-cli-0.26.5" -type d | head -1)
if [ -z "$CLI_SRC" ]; then
    echo "Error: tree-sitter-cli-0.26.5 not found in cargo registry."
    exit 1
fi

echo "Found generate source: $GENERATE_SRC"
echo "Found CLI source: $CLI_SRC"

# Set up build directory
mkdir -p "$BUILD_DIR"

# Copy sources (only if not already present or if source is newer)
if [ ! -d "$BUILD_DIR/tree-sitter-generate" ] || [ "$GENERATE_SRC/src/render.rs" -nt "$BUILD_DIR/tree-sitter-generate/src/render.rs" ]; then
    echo "Copying tree-sitter-generate source..."
    rm -rf "$BUILD_DIR/tree-sitter-generate"
    cp -r "$GENERATE_SRC" "$BUILD_DIR/tree-sitter-generate"
fi

if [ ! -d "$BUILD_DIR/tree-sitter-cli" ] || [ "$CLI_SRC/Cargo.toml" -nt "$BUILD_DIR/tree-sitter-cli/Cargo.toml" ]; then
    echo "Copying tree-sitter-cli source..."
    rm -rf "$BUILD_DIR/tree-sitter-cli"
    cp -r "$CLI_SRC" "$BUILD_DIR/tree-sitter-cli"
fi

# Apply the threshold patch
RENDER_RS="$BUILD_DIR/tree-sitter-generate/src/render.rs"
CURRENT=$(grep -oP 'const SMALL_STATE_THRESHOLD: usize = \K[0-9]+' "$RENDER_RS")
if [ "$CURRENT" != "$THRESHOLD" ]; then
    echo "Patching SMALL_STATE_THRESHOLD: $CURRENT → $THRESHOLD"
    sed -i "s/const SMALL_STATE_THRESHOLD: usize = $CURRENT;/const SMALL_STATE_THRESHOLD: usize = $THRESHOLD;/" "$RENDER_RS"
else
    echo "Already patched to $THRESHOLD"
fi

# Create workspace Cargo.toml with patch override
cat > "$BUILD_DIR/Cargo.toml" << 'CARGO_EOF'
[workspace]
members = ["tree-sitter-cli"]
resolver = "2"

[patch.crates-io]
tree-sitter-generate = { path = "./tree-sitter-generate" }
CARGO_EOF

# Build
echo "Building patched CLI (this takes ~2 minutes)..."
cd "$BUILD_DIR"
cargo build --release -p tree-sitter-cli 2>&1

BINARY="$BUILD_DIR/target/release/tree-sitter"
if [ -f "$BINARY" ]; then
    echo ""
    echo "=== Build successful ==="
    echo "Binary: $BINARY"
    echo "Size: $(du -h "$BINARY" | cut -f1)"
    echo ""
    echo "Usage: $BINARY generate --abi=14"
else
    echo "Error: Build failed — binary not found"
    exit 1
fi
