#!/bin/sh
# fetch.sh — reconstruct third-party sources from pinned upstream commits.
#
# The public repository does not vendor the OMG specification materials, the
# OMG pilot implementation, or the standard model library derived from it. This
# script rebuilds them locally from the upstream URLs and git commits pinned in
# manifest.toml, verifying every item against a recorded SHA-256.
#
#   ./tools/fetch-references/fetch.sh verify   # check an existing tree (no network)
#   ./tools/fetch-references/fetch.sh          # reconstruct the tree (default mode)
#   ./tools/fetch-references/fetch.sh fetch    # the same thing, named explicitly
#   ./tools/fetch-references/fetch.sh list     # print the pinned inventory
#   ./tools/fetch-references/fetch.sh hash P   # maintenance: hash a file or directory
#
# Exit status is 0 only when every selected item matches its pinned checksum.
#
# Requirements: POSIX sh, git, curl, awk, sed, sort, find, mv, mkdir, rmdir,
# and ONE of sha256sum (GNU coreutils) or shasum (Perl; the macOS default).
#:usage-end  — `--help` prints the comment block above this marker.
#
# PORTABILITY. This runs on GitHub's ubuntu, macos and windows runners, so it
# sticks to POSIX constructs on purpose: no GNU-only flags, explicit space/tab
# classes rather than POSIX bracket classes in awk patterns, no `cp -R src/.`,
# no `find -empty -delete`. See the notes at each site. SYSML_FETCH_SHA_TOOL
# forces one hash backend so both paths stay testable on a machine that has
# both (selftest.sh exercises each).

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
MANIFEST="$SCRIPT_DIR/manifest.toml"

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_OK=$(printf '\033[32m'); C_BAD=$(printf '\033[31m')
    C_WARN=$(printf '\033[33m'); C_DIM=$(printf '\033[2m'); C_OFF=$(printf '\033[0m')
else
    C_OK=''; C_BAD=''; C_WARN=''; C_DIM=''; C_OFF=''
fi

info() { printf '%s\n' "$*" >&2; }
warn() { printf '%swarning:%s %s\n' "$C_WARN" "$C_OFF" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$C_BAD" "$C_OFF" "$*" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Hashing
#
# Single files hash with plain SHA-256. Directories use the aggregate-hash
# algorithm documented in references/sysmlv2/spec-drop.toml, so a directory
# hash produced here is directly comparable with one recorded there: for every
# file under the directory, sorted by BYTE ORDER of the forward-slash relative
# path, feed `<relpath> NUL <lowercase-hex-sha256-of-file> LF` into one SHA-256.
# ---------------------------------------------------------------------------

# ONE backend, resolved once, used by both entry points. GNU runners have
# sha256sum; macOS runners have only shasum. Both print `<hex>  <name>`, so the
# digest is everything before the first space — taken with a parameter
# expansion rather than `cut`, which halves the process count on the 4210-file
# pilot tree and drops one more command from the dependency set.
#
# SYSML_FETCH_SHA_TOOL pins the backend. Its only purpose is testing: a Linux
# box has both tools and would otherwise never exercise the macOS path.

case "${SYSML_FETCH_SHA_TOOL:-}" in
    '')         if command -v sha256sum >/dev/null 2>&1; then SHA_TOOL=sha256sum
                elif command -v shasum >/dev/null 2>&1; then SHA_TOOL=shasum
                else SHA_TOOL=''
                fi ;;
    sha256sum)  SHA_TOOL=sha256sum ;;
    shasum)     SHA_TOOL=shasum ;;
    *) die "SYSML_FETCH_SHA_TOOL must be 'sha256sum' or 'shasum', got '$SYSML_FETCH_SHA_TOOL'" ;;
esac

command -v "$SHA_TOOL" >/dev/null 2>&1 \
    || die "no SHA-256 tool found: install coreutils (sha256sum) or perl (shasum)"

case "$SHA_TOOL" in
    sha256sum) SHA_ARGV='sha256sum' ;;
    shasum)    SHA_ARGV='shasum -a 256' ;;
esac

# shellcheck disable=SC2086  # SHA_ARGV must word-split; it is an argv, not a path.
sha256_run() { $SHA_ARGV "$@"; }

sha256_of() {
    # `--` keeps a path beginning with '-' from being read as an option. Both
    # backends honour it: GNU getopt does, and shasum's Getopt::Long does.
    _sha_out=$(sha256_run -- "$1") || return 1
    printf '%s' "${_sha_out%% *}"
}

sha256_stdin() {
    _sha_out=$(sha256_run) || return 1
    printf '%s' "${_sha_out%% *}"
}

hash_dir() {
    # $1 = directory. Prints the aggregate hash, or the empty string if absent.
    #
    # The NUL separator is load-bearing: it is what the Rust provenance gate
    # feeds, so the two implementations agree only if `\000` really emits a
    # zero byte here. Every POSIX printf does, but selftest.sh proves it
    # against a fixed two-file fixture rather than assuming.
    #
    # The tool is invoked in BATCHES via xargs rather than once per file. That
    # is not premature optimisation: macOS has no sha256sum, and its shasum is
    # a Perl script whose interpreter startup dominates — measured ~9x slower
    # per file, which on the 4210-file pilot tree is about a minute per hash
    # pass, and a fetch hashes each tree twice. Batching makes the macOS cost
    # comparable to the Linux one.
    #
    # The join is POSITIONAL: both backends emit one line per argument in
    # argument order, so output line N belongs to input path N. Reading the
    # path back out of the output would be wrong — GNU sha256sum escapes names
    # containing a backslash or newline and prefixes the line with '\', while
    # shasum does not, so the two would disagree on exactly the names most
    # likely to matter. A count mismatch is a hard error rather than a silent
    # mis-pairing.
    [ -d "$1" ] || { printf ''; return 0; }

    _hd_list=$(mktemp "${TMPDIR:-/tmp}/fetch-refs-list.XXXXXX") || return 1
    _hd_sums=$(mktemp "${TMPDIR:-/tmp}/fetch-refs-sums.XXXXXX") || {
        rm -f "$_hd_list"; return 1; }

    ( CDPATH= cd -- "$1" || exit 1
      find . -type f | sed 's|^\./||' | LC_ALL=C sort ) > "$_hd_list" || {
        rm -f "$_hd_list" "$_hd_sums"; return 1; }

    if [ -s "$_hd_list" ]; then
        # shellcheck disable=SC2086
        ( CDPATH= cd -- "$1" || exit 1
          tr '\n' '\000' < "$_hd_list" | xargs -0 $SHA_ARGV -- ) > "$_hd_sums" || {
            warn "hashing failed under $1"
            rm -f "$_hd_list" "$_hd_sums"; return 1; }
    else
        : > "$_hd_sums"
    fi

    _hd_n_paths=$(wc -l < "$_hd_list" | tr -d ' ')
    _hd_n_sums=$(wc -l < "$_hd_sums" | tr -d ' ')
    if [ "$_hd_n_paths" != "$_hd_n_sums" ]; then
        warn "hash count mismatch under $1: $_hd_n_paths files, $_hd_n_sums digests"
        rm -f "$_hd_list" "$_hd_sums"
        return 1
    fi

    {
        exec 3< "$_hd_sums"
        while IFS= read -r _hd_rel; do
            IFS= read -r _hd_line <&3 || break
            printf '%s\000%s\n' "$_hd_rel" "${_hd_line%% *}"
        done < "$_hd_list"
        exec 3<&-
    } | sha256_stdin

    rm -f "$_hd_list" "$_hd_sums"
}

hash_path() {
    if [ -d "$1" ]; then hash_dir "$1"
    elif [ -f "$1" ]; then sha256_of "$1"
    else printf ''
    fi
}

describe_size() {
    if [ -d "$1" ]; then printf '%s files' "$(find "$1" -type f | wc -l | tr -d ' ')"
    elif [ -f "$1" ]; then printf '1 file'
    else printf 'absent'
    fi
}

# ---------------------------------------------------------------------------
# Manifest parsing
#
# manifest.toml is deliberately restricted to a subset this awk program can
# read exactly: `[[item]]` records containing `key = "string"`, `key = integer`
# and `key = ["a", "b"]` array-of-string lines. No nested tables, no multi-line
# values, no comments after a value. Keeping the subset small is what lets the
# bootstrap step stay dependency-free — it must run before anything compiles.
#
# Emits one line per item, arrays joined with '|', columns separated by US
# (0x1f) rather than tab — tab is an IFS whitespace character, so `read` would
# collapse runs of it and silently shift every column after an empty field.
#   id kind dest repo commit url sha256 upstream_sha256 patch files include
#   exclude license base subdir rename
#
# New columns are appended, never inserted: every `read` below is positional.
# ---------------------------------------------------------------------------

FS_US=$(printf '\037')

parse_manifest() {
    awk -v S="$FS_US" '
        function flush() {
            if (id == "") return
            printf "%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s\n",
                id, S, kind, S, dest, S, repo, S, commit, S, url, S, sha256, S,
                upstream, S, patch, S, files, S, include, S, exclude, S, license, S,
                base, S, subdir, S, rename
            id=""; kind=""; dest=""; repo=""; commit=""; url=""; sha256=""
            upstream=""; patch=""; files=""; include=""; exclude=""; license=""
            base=""; subdir=""; rename=""
        }
        /^[ \t]*#/ { next }
        /^[ \t]*\[\[item\]\][ \t]*$/ { flush(); next }
        /^[ \t]*[A-Za-z0-9_]+[ \t]*=/ {
            eq = index($0, "=")
            key = substr($0, 1, eq - 1)
            val = substr($0, eq + 1)
            gsub(/^[ \t]+|[ \t]+$/, "", key)
            gsub(/^[ \t]+|[ \t]+$/, "", val)
            if (substr(val, 1, 1) == "[") {          # array of strings
                gsub(/^\[|\][ \t]*$/, "", val)
                n = split(val, parts, ",")
                out = ""
                for (i = 1; i <= n; i++) {
                    p = parts[i]
                    gsub(/^[ \t]*"|"[ \t]*$/, "", p)
                    if (p == "") continue
                    out = (out == "") ? p : out "|" p
                }
                val = out
            } else {                                  # string or integer
                gsub(/^"|"$/, "", val)
            }
            if (key == "id") id = val
            else if (key == "kind") kind = val
            else if (key == "dest") dest = val
            else if (key == "repo") repo = val
            else if (key == "commit") commit = val
            else if (key == "url") url = val
            else if (key == "sha256") sha256 = val
            else if (key == "upstream_sha256") upstream = val
            else if (key == "patch") patch = val
            else if (key == "files") files = val
            else if (key == "include") include = val
            else if (key == "exclude") exclude = val
            else if (key == "license") license = val
            else if (key == "base") base = val
            else if (key == "subdir") subdir = val
            else if (key == "rename") rename = val
            next
        }
        END { flush() }
    ' "$MANIFEST"
}

# ---------------------------------------------------------------------------
# Include / exclude filtering
#
# Patterns are matched against the whole forward-slash relative path with shell
# glob semantics, in which `*` crosses `/`. So `*.log` matches `a/b/c.log`,
# `.vscode/*` matches everything under `.vscode`, and a plain path matches
# itself exactly.
# ---------------------------------------------------------------------------

matches_any() {
    _path=$1; _patterns=$2
    [ -n "$_patterns" ] || return 1
    _rest=$_patterns
    while [ -n "$_rest" ]; do
        case "$_rest" in
            *'|'*) _pat=${_rest%%|*}; _rest=${_rest#*|} ;;
            *)     _pat=$_rest;       _rest='' ;;
        esac
        # shellcheck disable=SC2254
        case "$_path" in $_pat) return 0 ;; esac
        case "$_path" in "$_pat"/*) return 0 ;; esac
    done
    return 1
}

prune_tree() {
    # $1 = tree root, $2 = include patterns, $3 = exclude patterns.
    _root=$1; _inc=$2; _exc=$3
    ( CDPATH= cd -- "$_root" || exit 1
      find . -type f | sed 's|^\./||' | while IFS= read -r rel; do
          if [ -n "$_inc" ] && ! matches_any "$rel" "$_inc"; then
              printf '%s\n' "$rel"; continue
          fi
          if matches_any "$rel" "$_exc"; then printf '%s\n' "$rel"; fi
      done ) | while IFS= read -r doomed; do
          rm -f -- "$_root/$doomed"
      done
    prune_empty_dirs "$_root"
}

# Drop directories left empty by pruning. `find -empty -delete` would be one
# line, but both flags are GNU/BSD extensions with differing edge cases, and
# `-delete` refuses some arguments outright. `rmdir` already means "remove
# only if empty", and `-depth` walks bottom-up so a directory emptied by the
# walk is itself removable in the same pass. Failures are expected (any
# non-empty directory) and are what we want ignored.
prune_empty_dirs() {
    find "$1" -depth -type d ! -path "$1" -exec rmdir {} + 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Directory restructuring
#
# Some vendored trees are not a verbatim subtree of their upstream: they are a
# subtree whose top-level directories were renamed, and in one case flattened.
# `rename` carries that transform as manifest DATA — a list of
# `<from>/=<to>/` directory-prefix rules — rather than as a special case in
# this script, so a second restructured item needs no code change.
#
# Rules are applied in listed order into a staging tree, so several sources may
# fold into one target (the three `Kernel Libraries/<group>/` directories all
# become `library.kernel/`). Anything the rules do not claim is an ERROR: a new
# upstream directory must be an explicit decision, not silently kept at its old
# path or silently dropped.
# ---------------------------------------------------------------------------

rename_tree() {
    # $1 = tree root, $2 = rules joined with '|', each `<from>/=<to>/`.
    _rt_root=$1; _rt_rules=$2
    [ -n "$_rt_rules" ] || return 0

    _rt_stage="$_rt_root.renamed"
    rm -rf "$_rt_stage"
    mkdir -p "$_rt_stage" || return 1

    _rest=$_rt_rules
    while [ -n "$_rest" ]; do
        case "$_rest" in
            *'|'*) _rule=${_rest%%|*}; _rest=${_rest#*|} ;;
            *)     _rule=$_rest;       _rest='' ;;
        esac
        case "$_rule" in
            *=*) ;;
            *) warn "malformed rename rule (no '='): $_rule"; rm -rf "$_rt_stage"; return 1 ;;
        esac
        _from=${_rule%%=*}
        _to=${_rule#*=}
        # Directory prefixes only. A non-slash-terminated rule would rename by
        # string prefix, which silently captures sibling names that merely
        # share a prefix.
        case "$_from" in */) ;; *) warn "rename source must end in '/': $_from"; rm -rf "$_rt_stage"; return 1 ;; esac
        case "$_to"   in */) ;; *) warn "rename target must end in '/': $_to";   rm -rf "$_rt_stage"; return 1 ;; esac
        if [ ! -d "$_rt_root/$_from" ]; then
            warn "rename source not present in the fetched tree: $_from"
            rm -rf "$_rt_stage"; return 1
        fi
        mkdir -p "$_rt_stage/$_to" || { rm -rf "$_rt_stage"; return 1; }
        # Move file by file, recreating parents. `cp -R src/. dst/` would be
        # shorter but GNU and BSD cp disagree about trailing `/.` and `/`
        # sources, and getting it wrong here produces a subtly misplaced tree
        # that only shows up as a checksum mismatch. Enumerate instead.
        ( CDPATH= cd -- "$_rt_root/$_from" || exit 1
          find . -type f | sed 's|^\./||' ) | while IFS= read -r _rel; do
            _parent=${_rel%/*}
            [ "$_parent" = "$_rel" ] || mkdir -p "$_rt_stage/$_to$_parent"
            mv "$_rt_root/$_from$_rel" "$_rt_stage/$_to$_rel"
        done
        rm -rf "$_rt_root/$_from"
    done

    # Anything still here matched no rule. Report paths relative to the root
    # by trimming the prefix in the shell — a `sed` expression built from a
    # path would break on any path containing the delimiter.
    _leftover=$(find "$_rt_root" -type f | head -5)
    if [ -n "$_leftover" ]; then
        warn "no rename rule covers these upstream paths:"
        printf '%s\n' "$_leftover" | while IFS= read -r _l; do
            warn "  ${_l#"$_rt_root"/}"
        done
        rm -rf "$_rt_stage"
        return 1
    fi

    rm -rf "$_rt_root"
    mv "$_rt_stage" "$_rt_root"
}

# ---------------------------------------------------------------------------
# Fetching
# ---------------------------------------------------------------------------

# Builds the tree under $1/src. Kept separate from fetch_git_item so the
# caller can clean up on either outcome without installing an EXIT trap —
# this runs inside the main loop's subshell, where an EXIT trap would clobber
# the inherited cleanup for the results file.
build_git_tree() {
    # $1 tmp, $2 repo, $3 commit, $4 include, $5 exclude, $6 patch,
    # $7 subdir, $8 rename
    _tmp=$1; _repo=$2; _commit=$3; _inc=$4; _exc=$5; _patch=$6
    _subdir=${7:-}; _rename=${8:-}

    info "  cloning $_repo @ ${_commit%"${_commit#????????}"}…"
    git init -q "$_tmp/src" || return 1
    git -C "$_tmp/src" remote add origin "$_repo" || return 1
    if ! git -C "$_tmp/src" fetch -q --depth 1 origin "$_commit" 2>/dev/null; then
        info "  ${C_DIM}shallow fetch of the pinned commit was refused; retrying full${C_OFF}"
        git -C "$_tmp/src" fetch -q origin || return 1
    fi
    git -C "$_tmp/src" checkout -q FETCH_HEAD || return 1
    rm -rf "$_tmp/src/.git"

    # Narrow to a subtree before anything else, so `include`, `exclude` and
    # `rename` are all written against the subtree's own paths.
    if [ -n "$_subdir" ]; then
        if [ ! -d "$_tmp/src/$_subdir" ]; then
            warn "subdir '$_subdir' is not present at the pinned commit"
            return 1
        fi
        mv "$_tmp/src/$_subdir" "$_tmp/subdir" || return 1
        rm -rf "$_tmp/src"
        mv "$_tmp/subdir" "$_tmp/src" || return 1
    fi

    prune_tree "$_tmp/src" "$_inc" "$_exc" || return 1
    rename_tree "$_tmp/src" "$_rename" || return 1

    # The patch is applied last, so its paths are the LOCAL ones — a reviewer
    # reads it against the tree they have, not against upstream's layout.
    if [ -n "$_patch" ]; then
        info "  applying $_patch"
        ( CDPATH= cd -- "$_tmp/src" && git apply --unsafe-paths -p1 "$SCRIPT_DIR/$_patch" ) || return 1
    fi
}

fetch_git_item() {
    # $1 dest_abs, $2 repo, $3 commit, $4 include, $5 exclude, $6 patch,
    # $7 subdir, $8 rename
    _fgi_dest=$1
    _fgi_tmp=$(mktemp -d "${TMPDIR:-/tmp}/fetch-refs.XXXXXX")

    if build_git_tree "$_fgi_tmp" "$2" "$3" "$4" "$5" "$6" "${7:-}" "${8:-}"; then
        rm -rf "$_fgi_dest"
        mkdir -p "$(dirname "$_fgi_dest")"
        mv "$_fgi_tmp/src" "$_fgi_dest"
        rm -rf "$_fgi_tmp"
        return 0
    fi

    rm -rf "$_fgi_tmp"
    return 1
}

fetch_url_item() {
    # $1 dest_abs, $2 url
    _dest=$1; _url=$2
    info "  downloading $_url"
    mkdir -p "$(dirname "$_dest")"
    _tmp="$_dest.download.$$"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$_tmp" "$_url"; then
        rm -f "$_tmp"
        return 1
    fi
    mv "$_tmp" "$_dest"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

usage() {
    # Read the header up to the marker rather than a hardcoded line range: the
    # range silently truncated the last paragraph the moment the header grew.
    awk 'NR == 1 { next }
         /^#:usage-end/ { exit }
         /^#/ { sub(/^#[ ]?/, ""); print; next }
         { exit }' "$0"
    cat <<'EOF'

Options:
  --root DIR     reconstruct every item under DIR instead of its declared base.
                 Item destinations are disjoint, so this relocates the whole
                 fetch set into one scratch directory — which is how a real
                 reconstruction is tested without touching the working tree.
  --item ID      restrict to matching manifest items (repeatable). Matching is
                 glob-based, so `--item kpar-*` selects every kpar item. A
                 value that matches no item is an error, not an empty run.
  --force        re-fetch items that already verify
  --keep-going   report every failure instead of stopping at the first
  -h, --help     show this message

Each item declares the base its `dest` is relative to. `base = "references"`
(the default) resolves under $SYSML_REFS_DIR when set, otherwise under
<repo>/references/sysmlv2 — the same resolution order the Rust build scripts
use, so a tree fetched here is found by `cargo build` with no extra config.
`base = "repo"` resolves under the repository root instead, for material the
runtime reads from outside references/ (the standard model library).
EOF
}

MODE=''
ROOT=''
FORCE=0
KEEP_GOING=0
ONLY=''

# A bare invocation reconstructs the tree. That is what a fresh clone needs and
# what the README and CONTRIBUTING quick starts invoke, so it is the default
# rather than an error.
if [ $# -eq 0 ]; then
    MODE=fetch
else
    case "$1" in
        verify|fetch|list|hash) MODE=$1; shift ;;
        -h|--help) usage; exit 0 ;;
        --*) MODE=fetch ;;
        *) die "unknown mode '$1' (expected verify, fetch, list or hash)" ;;
    esac
fi

if [ "$MODE" = hash ]; then
    [ $# -ge 1 ] || die "hash requires a path"
    for p in "$@"; do
        printf '%s  %s  (%s)\n' "$(hash_path "$p")" "$p" "$(describe_size "$p")"
    done
    exit 0
fi

while [ $# -gt 0 ]; do
    case "$1" in
        --root) [ $# -ge 2 ] || die "--root requires a value"; ROOT=$2; shift 2 ;;
        --item) [ $# -ge 2 ] || die "--item requires a value"
                ONLY="$ONLY|$2"; shift 2 ;;
        --force) FORCE=1; shift ;;
        --keep-going) KEEP_GOING=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option '$1'" ;;
    esac
done

[ -f "$MANIFEST" ] || die "manifest not found: $MANIFEST"

REFS_ROOT=${SYSML_REFS_DIR:-$REPO_ROOT/references/sysmlv2}
ONLY=${ONLY#|}

# Resolve one item's destination. `--root` overrides every base at once: item
# destinations are disjoint, so a single scratch directory is an unambiguous
# sandbox for a full reconstruction, and no item can write into the working
# tree during such a run.
dest_root_for() {
    if [ -n "$ROOT" ]; then printf '%s' "$ROOT"
    elif [ "$1" = repo ]; then printf '%s' "$REPO_ROOT"
    else printf '%s' "$REFS_ROOT"
    fi
}

# Every --item must select something. Without this check a typo'd or drifted
# name silently selects nothing, and an empty run reports "all 0 items
# verified" with exit 0 — a green no-op that in CI only surfaces downstream,
# where the missing tree looks like an unrelated build failure. Fail closed
# instead: an item name that matches no manifest record is a usage error.
if [ -n "$ONLY" ]; then
    ALL_IDS=$(parse_manifest | cut -d"$FS_US" -f1)
    UNMATCHED=''
    _rest=$ONLY
    while [ -n "$_rest" ]; do
        case "$_rest" in
            *'|'*) _pat=${_rest%%|*}; _rest=${_rest#*|} ;;
            *)     _pat=$_rest;       _rest='' ;;
        esac
        _hit=0
        for _id in $ALL_IDS; do
            if matches_any "$_id" "$_pat"; then _hit=1; break; fi
        done
        [ "$_hit" -eq 1 ] || UNMATCHED="$UNMATCHED $_pat"
    done
    if [ -n "$UNMATCHED" ]; then
        printf '%serror:%s no manifest item matches:%s\n' \
            "$C_BAD" "$C_OFF" "$UNMATCHED" >&2
        printf 'known items:\n' >&2
        for _id in $ALL_IDS; do printf '  %s\n' "$_id" >&2; done
        exit 2
    fi
fi

if [ "$MODE" = list ]; then
    printf '%-26s %-6s %-10s %s\n' ID KIND LICENSE PIN
    parse_manifest | while IFS="$FS_US" read -r id kind dest repo commit url sha256 upstream patch files include exclude license base subdir rename; do
        case "$kind" in
            git) pin="$repo @ $commit${subdir:+ :: $subdir}" ;;
            *)   pin="$url" ;;
        esac
        printf '%-26s %-6s %-10s %s\n' "$id" "$kind" "${license:--}" "$pin"
    done
    exit 0
fi

if [ -n "$ROOT" ]; then
    info "root override:   $ROOT (all items)"
else
    info "references root: $REFS_ROOT"
    info "repository root: $REPO_ROOT"
fi
info "manifest:        $MANIFEST"
info ""

RESULTS=$(mktemp "${TMPDIR:-/tmp}/fetch-refs-results.XXXXXX")
trap 'rm -f "$RESULTS"' EXIT INT TERM

parse_manifest | while IFS="$FS_US" read -r id kind dest repo commit url sha256 upstream patch files include exclude license base subdir rename; do
    [ -n "$id" ] || continue
    if [ -n "$ONLY" ] && ! matches_any "$id" "$ONLY"; then continue; fi

    dest_abs="$(dest_root_for "$base")/$dest"
    actual=$(hash_path "$dest_abs")

    if [ "$MODE" = fetch ]; then
        if [ "$actual" = "$sha256" ] && [ "$FORCE" -eq 0 ]; then
            printf '%s %-40s %sup to date%s\n' "${C_OK}✓${C_OFF}" "$id" "$C_DIM" "$C_OFF"
            printf 'ok\n' >> "$RESULTS"
            continue
        fi
        info "$id"
        ok=1
        case "$kind" in
            git)  fetch_git_item "$dest_abs" "$repo" "$commit" "$include" "$exclude" "$patch" "$subdir" "$rename" || ok=0 ;;
            file) fetch_url_item "$dest_abs" "$url" || ok=0 ;;
            *)    warn "item '$id' has unsupported kind '$kind'"; ok=0 ;;
        esac
        if [ "$ok" -eq 0 ]; then
            printf '%s %-40s %sfetch failed%s\n' "${C_BAD}✗${C_OFF}" "$id" "$C_BAD" "$C_OFF"
            printf 'fail\n' >> "$RESULTS"
            [ "$KEEP_GOING" -eq 1 ] || break
            continue
        fi
        actual=$(hash_path "$dest_abs")
    fi

    if [ -z "$actual" ]; then
        printf '%s %-40s %smissing%s\n' "${C_BAD}✗${C_OFF}" "$id" "$C_BAD" "$C_OFF"
        printf 'fail\n' >> "$RESULTS"
        [ "$KEEP_GOING" -eq 1 ] || [ "$MODE" = verify ] || break
    elif [ "$actual" = "$sha256" ]; then
        printf '%s %-40s %s%s%s\n' "${C_OK}✓${C_OFF}" "$id" "$C_DIM" "$(describe_size "$dest_abs")" "$C_OFF"
        printf 'ok\n' >> "$RESULTS"
    else
        printf '%s %-40s %schecksum mismatch%s\n' "${C_BAD}✗${C_OFF}" "$id" "$C_BAD" "$C_OFF"
        printf '    expected %s\n' "$sha256"
        printf '    actual   %s\n' "$actual"
        printf 'fail\n' >> "$RESULTS"
        [ "$KEEP_GOING" -eq 1 ] || [ "$MODE" = verify ] || break
    fi
done

total=$(wc -l < "$RESULTS" | tr -d ' ')
failed=$(grep -c '^fail$' "$RESULTS" || true)
passed=$((total - failed))

# Belt and braces alongside the --item validation above: reporting success
# for zero items is never a correct outcome, so treat it as a failure rather
# than printing a reassuring "all 0 items verified".
if [ "$total" -eq 0 ]; then
    die "no items were processed — the manifest is empty or unreadable: $MANIFEST"
fi

info ""
if [ "$failed" -eq 0 ]; then
    info "${C_OK}all $total items verified${C_OFF}"
    exit 0
fi
info "${C_BAD}$failed of $total items failed${C_OFF} ($passed ok)"
if [ "$MODE" = verify ]; then
    info "run '$0 fetch' to reconstruct the failing items from upstream"
fi
exit 1
