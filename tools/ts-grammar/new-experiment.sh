#!/usr/bin/env bash
# new-experiment.sh - Create a worktree for a tree-sitter grammar experiment.
#
# Usage:
#   tools/ts-grammar/new-experiment.sh <name> [base-branch]
#
# Creates .worktrees/grammar-<name> on a fresh branch grammar-<name>
# branched off architectural-cleanup (or [base-branch] if supplied),
# then seeds parser.c via cache-build.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

if [[ $# -lt 1 ]]; then
    echo "usage: $0 <name> [base-branch]" >&2
    exit 1
fi

NAME="$1"
BASE_BRANCH="${2:-architectural-cleanup}"
BRANCH="grammar-$NAME"
WORKTREE="$REPO_ROOT/.worktrees/grammar-$NAME"

if [[ -e "$WORKTREE" ]]; then
    echo "error: worktree path already exists: $WORKTREE" >&2
    exit 1
fi

cd "$REPO_ROOT"

# Verify base branch exists.
if ! git show-ref --verify --quiet "refs/heads/$BASE_BRANCH"; then
    echo "error: base branch '$BASE_BRANCH' does not exist locally" >&2
    exit 1
fi

# Verify the new branch name doesn't already exist.
if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
    echo "error: branch '$BRANCH' already exists" >&2
    exit 1
fi

echo "creating worktree: $WORKTREE on branch $BRANCH (off $BASE_BRANCH)"
mkdir -p "$REPO_ROOT/.worktrees"
git worktree add -b "$BRANCH" "$WORKTREE" "$BASE_BRANCH"

# Seed parser.c in the new worktree from the shared cache.
echo ""
echo "seeding parser.c from .tree-sitter-cache..."
(
    cd "$WORKTREE"
    if [[ -x tools/ts-grammar/cache-build.sh ]]; then
        tools/ts-grammar/cache-build.sh
    else
        echo "warn: cache-build.sh not found in worktree (this is expected if the script" >&2
        echo "      isn't yet on $BASE_BRANCH). Falling back to a manual seed:" >&2
        # The cache is shared across worktrees (lives at repo-root/.tree-sitter-cache),
        # so even before scripts land on the branch we can copy from the most recent
        # entry if we know which hash matches. Skip - user will need to re-merge.
        echo "      run 'tools/ts-grammar/cache-build.sh' once it's available." >&2
    fi
)

cat <<EOF

================================================================
worktree ready: $WORKTREE
branch:         $BRANCH

next steps:
  cd .worktrees/grammar-$NAME
  # edit tree-sitter/rules/*.js or grammar.js
  tools/ts-grammar/measure.sh                 # quick metrics check (sub-second)
  tools/ts-grammar/cache-build.sh             # regenerate + cache parser.c (~50 min if MISS)
  cargo build --release -p sysml-parser-incremental
  cargo test  --release -p sysml-parser-incremental
  # if green:
  git add <files-by-name>
  git commit
================================================================
EOF
