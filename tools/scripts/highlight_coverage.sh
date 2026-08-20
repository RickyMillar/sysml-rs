#!/usr/bin/env bash
set -euo pipefail

# highlight_coverage.sh — Validates all grammar keywords appear in both
# highlights.scm files (tree-sitter and Zed extension).
#
# Usage:
#   ./scripts/highlight_coverage.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

NODE_TYPES="$WORKSPACE/sysml-ts/tree-sitter/src/node-types.json"
TS_HIGHLIGHTS="$WORKSPACE/sysml-ts/tree-sitter/queries/highlights.scm"
ZED_HIGHLIGHTS="$WORKSPACE/sysml-lsp-zed-extension/languages/sysml/highlights.scm"

# Color helpers
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}Highlight Coverage Check${NC}"
echo "========================"
echo ""

# Use python3 for reliable JSON + regex parsing
export NODE_TYPES TS_HIGHLIGHTS ZED_HIGHLIGHTS
python3 << 'PYEOF'
import json
import os
import re
import sys

node_types_path = os.environ["NODE_TYPES"]
ts_highlights_path = os.environ["TS_HIGHLIGHTS"]
zed_highlights_path = os.environ["ZED_HIGHLIGHTS"]

# 1. Extract keyword nodes from node-types.json
# Keywords are entries where "named" is false and "type" is all alphabetic
with open(node_types_path) as f:
    node_types = json.load(f)

grammar_keywords = set()
for entry in node_types:
    if not entry.get("named", True) and entry.get("type", "").isalpha():
        grammar_keywords.add(entry["type"])

# 2. Extract keywords from a highlights.scm file
# We look for string literals that are captured by any @capture group
def extract_highlight_keywords(path):
    with open(path) as f:
        content = f.read()

    keywords = set()
    # Match patterns like: ["keyword1" "keyword2" ...] @capture
    # Also handle single-string patterns: "keyword" @capture
    # Any capture group counts (not just @keyword — also @constant.builtin, @variable.builtin, etc.)

    # Find all [...] @capture blocks
    bracket_pattern = re.compile(
        r'\[([^\]]*)\]\s*@\w+(?:\.\w+)*',
        re.DOTALL
    )
    for match in bracket_pattern.finditer(content):
        block = match.group(1)
        for word in re.findall(r'"(\w+)"', block):
            keywords.add(word)

    # Also handle bare patterns: "keyword" @capture
    bare_pattern = re.compile(
        r'"(\w+)"\s*@\w+(?:\.\w+)*'
    )
    for match in bare_pattern.finditer(content):
        keywords.add(match.group(1))

    return keywords

ts_keywords = extract_highlight_keywords(ts_highlights_path)
zed_keywords = extract_highlight_keywords(zed_highlights_path)

# 3. Compute differences
missing_ts = sorted(grammar_keywords - ts_keywords)
missing_zed = sorted(grammar_keywords - zed_keywords)

# 4. Report
print(f"Grammar keywords: {len(grammar_keywords)}")
print(f"Tree-sitter highlights.scm keywords: {len(ts_keywords)}")
print(f"Zed extension highlights.scm keywords: {len(zed_keywords)}")
print()

if missing_ts:
    print(f"Missing from tree-sitter highlights.scm ({len(missing_ts)}):")
    for kw in missing_ts:
        print(f'  "{kw}"')
else:
    print("Missing from tree-sitter highlights.scm: (none)")

print()

if missing_zed:
    print(f"Missing from Zed highlights.scm ({len(missing_zed)}):")
    for kw in missing_zed:
        print(f'  "{kw}"')
else:
    print("Missing from Zed highlights.scm: (none)")

print()

total_missing = len(missing_ts) + len(missing_zed)
if total_missing == 0:
    print("Result: PASS (full keyword coverage)")
    sys.exit(0)
else:
    parts = []
    if missing_ts:
        parts.append(f"{len(missing_ts)} keywords missing from tree-sitter")
    if missing_zed:
        parts.append(f"{len(missing_zed)} keywords missing from Zed extension")
    print(f"Result: FAIL ({', '.join(parts)})")
    sys.exit(1)
PYEOF
