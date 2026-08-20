#!/usr/bin/env bash
# cache-prune.sh - Evict old cache entries from .tree-sitter-cache/.
#
# Flags:
#   --older-than <duration>   e.g. 7d, 30d, 12h (default: 30d)
#   --keep-latest <N>         always retain the N most-recent entries (default: 3)
#   --dry-run                 print what would be deleted; do not delete
#
# Reasoning: each cache entry is ~50 MB. After a few weeks of grammar churn,
# stale entries dominate disk use. Default keeps the 3 most recent entries
# regardless of age, so a careless --older-than 1h can't wipe everything.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CACHE_DIR="$REPO_ROOT/.tree-sitter-cache"

OLDER_THAN="30d"
KEEP_LATEST="3"
DRY_RUN="0"

usage() {
    sed -n '2,12p' "$0"
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --older-than) OLDER_THAN="$2"; shift 2 ;;
        --keep-latest) KEEP_LATEST="$2"; shift 2 ;;
        --dry-run) DRY_RUN="1"; shift ;;
        -h|--help) usage 0 ;;
        *) echo "unknown flag: $1" >&2; usage 1 ;;
    esac
done

# Convert duration to seconds. Accept Nd, Nh, Nm, Ns (default: days).
duration_to_seconds() {
    local d="$1"
    local num="${d%[a-zA-Z]*}"
    local unit="${d##*[0-9]}"
    case "$unit" in
        d|"") echo $(( num * 86400 )) ;;
        h)    echo $(( num * 3600 )) ;;
        m)    echo $(( num * 60 )) ;;
        s)    echo "$num" ;;
        *) echo "error: bad duration '$d' (use Nd, Nh, Nm, Ns)" >&2; exit 1 ;;
    esac
}

CUTOFF_SEC="$(duration_to_seconds "$OLDER_THAN")"
NOW="$(date +%s)"

if [[ ! -d "$CACHE_DIR" ]]; then
    echo "cache dir $CACHE_DIR does not exist - nothing to prune"
    exit 0
fi

# List entries sorted by mtime (newest first).
mapfile -t ENTRIES < <(
    find "$CACHE_DIR" -mindepth 1 -maxdepth 1 -type d -printf '%T@\t%p\n' 2>/dev/null \
        | sort -rn \
        | cut -f2
)

if [[ ${#ENTRIES[@]} -eq 0 ]]; then
    echo "no cache entries found in $CACHE_DIR"
    exit 0
fi

TOTAL=${#ENTRIES[@]}
KEPT=0
DELETED=0
RECLAIMED=0

echo "cache: $TOTAL entries, keeping latest $KEEP_LATEST, evicting >${OLDER_THAN} old"

for i in "${!ENTRIES[@]}"; do
    ENTRY="${ENTRIES[$i]}"
    NAME="$(basename "$ENTRY")"

    if [[ $i -lt $KEEP_LATEST ]]; then
        KEPT=$((KEPT + 1))
        echo "  KEEP (recent #$((i+1))): $NAME"
        continue
    fi

    MTIME="$(stat -c '%Y' "$ENTRY" 2>/dev/null || stat -f '%m' "$ENTRY")"
    AGE_SEC=$((NOW - MTIME))

    if [[ $AGE_SEC -lt $CUTOFF_SEC ]]; then
        KEPT=$((KEPT + 1))
        echo "  KEEP (within window): $NAME (age ${AGE_SEC}s)"
        continue
    fi

    SIZE_KB="$(du -sk "$ENTRY" 2>/dev/null | cut -f1)"
    RECLAIMED=$((RECLAIMED + SIZE_KB))
    DELETED=$((DELETED + 1))

    if [[ "$DRY_RUN" == "1" ]]; then
        echo "  DRY-DELETE: $NAME (${SIZE_KB} KB, age ${AGE_SEC}s)"
    else
        echo "  DELETE: $NAME (${SIZE_KB} KB, age ${AGE_SEC}s)"
        rm -rf "$ENTRY"
    fi
done

RECLAIMED_MB=$((RECLAIMED / 1024))
if [[ "$DRY_RUN" == "1" ]]; then
    echo "would reclaim: ${RECLAIMED_MB} MB across $DELETED entries (kept $KEPT)"
else
    echo "reclaimed: ${RECLAIMED_MB} MB across $DELETED entries (kept $KEPT)"
fi
