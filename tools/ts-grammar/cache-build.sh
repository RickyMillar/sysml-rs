#!/usr/bin/env bash
# cache-build.sh - Content-addressable cache for tree-sitter parser.c
#
# Computes sha256(grammar.js); if cached, links the cached parser.c into place.
# Otherwise runs `tree-sitter generate --abi 14` and stores the result.
#
# Reasoning: a clean tree-sitter generate + parser.c compile costs ~50 minutes.
# Many grammar experiments will reach the same intermediate grammar.js; pay
# the build cost ONCE per unique hash, share across worktrees / agents.

set -euo pipefail

# Resolve the repo root regardless of where this is invoked from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

GRAMMAR_DIR="$REPO_ROOT/crates/lang/sysml-parser-incremental/tree-sitter"
GRAMMAR_JS="$GRAMMAR_DIR/grammar.js"
PARSER_C="$GRAMMAR_DIR/src/parser.c"
CACHE_DIR="$REPO_ROOT/.tree-sitter-cache"

if [[ ! -f "$GRAMMAR_JS" ]]; then
    echo "error: grammar.js not found at $GRAMMAR_JS" >&2
    exit 1
fi

# Hash the entire grammar surface: grammar.js plus everything it requires
# (rules/, helpers/, generated/). A change to any of these forces regen.
hash_grammar() {
    # Sorted concatenation of grammar surface files, then sha256.
    {
        find "$GRAMMAR_DIR/rules" -type f -name '*.js' 2>/dev/null | sort
        find "$GRAMMAR_DIR/helpers" -type f -name '*.js' 2>/dev/null | sort
        find "$GRAMMAR_DIR/generated" -type f -name '*.js' 2>/dev/null | sort
        echo "$GRAMMAR_JS"
    } | xargs cat 2>/dev/null | sha256sum | cut -d' ' -f1
}

GRAMMAR_HASH="$(hash_grammar)"
CACHE_ENTRY="$CACHE_DIR/$GRAMMAR_HASH"
CACHED_PARSER="$CACHE_ENTRY/parser.c"
CACHED_GRAMMAR_JSON="$CACHE_ENTRY/grammar.json"
CACHED_NODE_TYPES="$CACHE_ENTRY/node-types.json"

mkdir -p "$CACHE_DIR"

if [[ -f "$CACHED_PARSER" ]]; then
    echo "cache HIT: $GRAMMAR_HASH (saved ~50 min)"
    mkdir -p "$GRAMMAR_DIR/src"
    # Copy (not symlink) to avoid surprises with tools that rewrite the file.
    cp "$CACHED_PARSER" "$PARSER_C"
    [[ -f "$CACHED_GRAMMAR_JSON" ]] && cp "$CACHED_GRAMMAR_JSON" "$GRAMMAR_DIR/src/grammar.json"
    [[ -f "$CACHED_NODE_TYPES" ]] && cp "$CACHED_NODE_TYPES" "$GRAMMAR_DIR/src/node-types.json"
    # Touch cache entry so prune --keep-latest treats it as recently used.
    touch "$CACHE_ENTRY"
    exit 0
fi

echo "cache MISS: $GRAMMAR_HASH - acquiring generate lock..."

# Serialize tree-sitter generate across worktrees. Concurrent generates
# (5+) blow past 54Gi RAM + 8Gi swap and trigger OOM kills. The lock is
# coarse-grained: one generate at a time, system-wide. Cache HITs above
# bypass this entirely.
LOCK_FILE="$CACHE_DIR/.generate.lock"
mkdir -p "$CACHE_DIR"
exec 200>"$LOCK_FILE"
echo "waiting for lock (other generate may be in flight)..."
flock 200
echo "lock acquired"

# Re-check cache: another agent may have finished the same hash while we waited.
if [[ -f "$CACHED_PARSER" ]]; then
    echo "cache HIT after lock wait: $GRAMMAR_HASH (saved ~50 min)"
    mkdir -p "$GRAMMAR_DIR/src"
    cp "$CACHED_PARSER" "$PARSER_C"
    [[ -f "$CACHED_GRAMMAR_JSON" ]] && cp "$CACHED_GRAMMAR_JSON" "$GRAMMAR_DIR/src/grammar.json"
    [[ -f "$CACHED_NODE_TYPES" ]] && cp "$CACHED_NODE_TYPES" "$GRAMMAR_DIR/src/node-types.json"
    touch "$CACHE_ENTRY"
    exit 0
fi

echo "running tree-sitter generate (~50 min)..."

# Require tree-sitter-cli to be present and the correct version.
if ! command -v tree-sitter >/dev/null 2>&1; then
    echo "error: tree-sitter not found in PATH. Install with:" >&2
    echo "    npm install -g tree-sitter-cli@0.26.5" >&2
    exit 1
fi

TS_VERSION="$(tree-sitter --version 2>&1 | head -1)"
echo "using $TS_VERSION"

# Run generate from the grammar directory.
cd "$GRAMMAR_DIR"
tree-sitter generate --abi 14

if [[ ! -f "$PARSER_C" ]]; then
    echo "error: tree-sitter generate did not produce $PARSER_C" >&2
    exit 1
fi

# Cache the generated artifacts.
mkdir -p "$CACHE_ENTRY"
cp "$PARSER_C" "$CACHED_PARSER"
[[ -f "$GRAMMAR_DIR/src/grammar.json" ]] && cp "$GRAMMAR_DIR/src/grammar.json" "$CACHED_GRAMMAR_JSON"
[[ -f "$GRAMMAR_DIR/src/node-types.json" ]] && cp "$GRAMMAR_DIR/src/node-types.json" "$CACHED_NODE_TYPES"

# Stash provenance for diagnostics.
{
    echo "grammar_hash: $GRAMMAR_HASH"
    echo "generated_at: $(date -Iseconds)"
    echo "tree_sitter: $TS_VERSION"
    echo "parser_c_size: $(stat -c '%s' "$CACHED_PARSER" 2>/dev/null || stat -f '%z' "$CACHED_PARSER")"
} > "$CACHE_ENTRY/provenance.txt"

SIZE_MB="$(du -m "$CACHED_PARSER" | cut -f1)"
echo "cached: $CACHE_ENTRY/parser.c (${SIZE_MB} MB)"
echo "cache MISS: $GRAMMAR_HASH - generated parser.c, cached for next run"
