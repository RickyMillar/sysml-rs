#!/bin/sh
# selftest.sh — checks fetch.sh itself, not the reference tree.
#
# Everything here is offline and takes a second or two, so it is safe to run
# in CI alongside the real `verify`. It exists because the failure mode that
# matters most for this script is a silent one: a check that passes without
# having checked anything.
#
#   ./tools/fetch-references/selftest.sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
FETCH="$SCRIPT_DIR/fetch.sh"
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)

pass=0
fail=0

ok()   { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
bad()  { fail=$((fail + 1)); printf 'FAIL %s\n' "$1"; [ $# -lt 2 ] || printf '     %s\n' "$2"; }

# ---------------------------------------------------------------------------
# 1. Syntax
# ---------------------------------------------------------------------------

if sh -n "$FETCH"; then ok "fetch.sh parses"; else bad "fetch.sh parses"; fi

# `--help` renders the header comment block. It used to do that with a
# hardcoded line range, which silently dropped the last paragraph as soon as
# the header grew — the dependency list, of all things. It now reads to a
# marker; this asserts the last header line still reaches the output.
last_header=$(awk 'NR == 1 { next }
                   /^#:usage-end/ { exit }
                   /^#/ { sub(/^#[ ]?/, ""); if ($0 != "") last = $0; next }
                   { exit }
                   END { print last }' "$FETCH")

if [ -z "$last_header" ]; then
    bad "--help renders the whole header block" "could not find the header marker in fetch.sh"
elif "$FETCH" --help 2>&1 | grep -qF -- "$last_header"; then
    ok "--help renders the whole header block"
else
    bad "--help renders the whole header block" "last header line missing: $last_header"
fi

# ---------------------------------------------------------------------------
# 1b. REGRESSION: no unbraced expansion abuts an identifier-continuation byte
#
# This is the bug that took both macOS legs down: an expansion written as
# `$C_OK` with a ✓ glyph immediately after it, inside one double-quoted word.
# That is fine on a modern bash, which ends the variable name at the multibyte
# character. bash 3.2 — which is what /bin/sh and /bin/bash still are on macOS
# — reads the UTF-8 continuation bytes INTO the name, so the expansion becomes
# an unset `C_OK<garbage>` and `set -u` aborts the run. It fails only on the
# branch that prints a tick, which is why every earlier check passed.
#
# Comment lines are skipped: a hazard inside a comment never executes, and
# this file has to be able to describe the construct in prose.
#
# The rule enforced here is the general one, not just for the colour
# variables: an unbraced `$NAME` may only be followed by a character that
# cannot possibly continue an identifier. Anything else must be braced.
#
# Implemented with awk and an explicit safe-character set rather than a byte
# class, because grep implementations disagree about whether a multibyte glyph
# is one printable character or several non-printable bytes — a `[^[:print:]]`
# form silently matched nothing under a Unicode-aware grep, which would have
# shipped a check that never checks.
# ---------------------------------------------------------------------------

# Passed through the ENVIRONMENT, not `awk -v`: -v runs its value through
# escape processing a second time, which silently ate the backslash out of
# this set and turned every `"$var\n"` in the repo into a false positive.
# ENVIRON delivers the bytes verbatim.
FETCH_SAFE_AFTER=$(printf ' \t"'"'"'`$(){}[]<>|&;,.:=+*/\\?#%%@^~-!')
export FETCH_SAFE_AFTER

scan_unbraced() {
    awk '
        BEGIN { SAFE = ENVIRON["FETCH_SAFE_AFTER"] }
        function unsafe_tail(line,   rest, ch) {
            rest = line
            while (match(rest, /\$[A-Za-z_][A-Za-z0-9_]*/)) {
                ch = substr(rest, RSTART + RLENGTH, 1)
                if (ch != "" && index(SAFE, ch) == 0)
                    return substr(rest, RSTART, RLENGTH + 1)
                rest = substr(rest, RSTART + RLENGTH)
            }
            return ""
        }
        /^[ \t]*#/ { next }
        { hit = unsafe_tail($0); if (hit != "") printf "%s:%d: %s\n", FILENAME, FNR, hit }
    ' "$@"
}

unbraced=$(scan_unbraced "$FETCH" "$0" 2>/dev/null || true)

# Prove the scanner still detects the exact construct that broke CI. A silent
# regex or locale change here would otherwise turn this into a no-op. The
# probe is the literal bytes of `"$C_OK<U+2713>$C_OFF"`.
probe=$(mktemp "${TMPDIR:-/tmp}/fetch-selftest-probe.XXXXXX")
# The '%s' placeholders keep the hazard out of THIS file's own source, so the
# scanner does not flag its own probe generator.
printf 'printf "%%s" "%sC_OK\342\234\223%sC_OFF"\n' '$' '$' > "$probe"
probe_hit=$(scan_unbraced "$probe" 2>/dev/null || true)
rm -f "$probe"

if [ -z "$probe_hit" ]; then
    bad "no unbraced expansion abuts an identifier byte (bash 3.2)" \
        "the scanner failed to flag the known-bad construct — it is not checking anything"
elif [ -z "$unbraced" ]; then
    ok "no unbraced expansion abuts an identifier byte (bash 3.2)"
else
    bad "no unbraced expansion abuts an identifier byte (bash 3.2)" "$unbraced"
fi

# ---------------------------------------------------------------------------
# 1a. The hash backends agree, and each produces the known digest
#
# GNU runners have sha256sum; macOS runners have only shasum. The two paths
# must be interchangeable, and neither may quietly produce a differently
# formatted digest — a trailing filename or a truncated hex string would turn
# every item into a checksum mismatch on one platform only.
#
# The directory case matters more than the file case: it exercises the NUL
# separator, the byte-order sort, and the pipeline, which is where a portable-
# looking script is most likely to diverge. Both expected values below were
# computed from the algorithm definition, not captured from a run.
# ---------------------------------------------------------------------------

HASH_FIXTURE=$(mktemp -d "${TMPDIR:-/tmp}/fetch-selftest.XXXXXX")
trap 'rm -rf "$HASH_FIXTURE"' EXIT INT TERM

printf 'abc' > "$HASH_FIXTURE/file"
# sha256("abc")
EXPECT_FILE=ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad

mkdir -p "$HASH_FIXTURE/tree/sub"
printf 'abc' > "$HASH_FIXTURE/tree/a.txt"
printf 'abc' > "$HASH_FIXTURE/tree/sub/b.txt"
# sha256 of the literal byte string
#   "a.txt" NUL <sha256(abc)> LF "sub/b.txt" NUL <sha256(abc)> LF
#
# HARDCODED on purpose. Deriving it here with the same `printf '\000'` the
# script uses would make the check vacuous: a shell that emitted nothing for
# \000 would produce a matching wrong answer on both sides and the test would
# pass while proving the opposite of what it claims.
EXPECT_TREE=fa36b450c4faee551040c4b79b56c0df1bff38ebc6bbec047495e37d86aea214

for tool in sha256sum shasum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'skip %s\n' "hash backend $tool (not installed)"
        continue
    fi
    got_file=$(SYSML_FETCH_SHA_TOOL="$tool" "$FETCH" hash "$HASH_FIXTURE/file" | cut -d' ' -f1)
    got_tree=$(SYSML_FETCH_SHA_TOOL="$tool" "$FETCH" hash "$HASH_FIXTURE/tree" | cut -d' ' -f1)
    if [ "$got_file" = "$EXPECT_FILE" ] && [ "$got_tree" = "$EXPECT_TREE" ]; then
        ok "hash backend $tool produces the known file and directory digests"
    else
        bad "hash backend $tool produces the known file and directory digests" \
            "file: want $EXPECT_FILE got $got_file; tree: want $EXPECT_TREE got $got_tree"
    fi
done

if SYSML_FETCH_SHA_TOOL=not-a-tool "$FETCH" hash "$HASH_FIXTURE/file" >/dev/null 2>&1; then
    bad "an unknown SYSML_FETCH_SHA_TOOL is rejected" "it was accepted"
else
    ok "an unknown SYSML_FETCH_SHA_TOOL is rejected"
fi

# ---------------------------------------------------------------------------
# 2. Manifest parses, and every record is well formed
#
# A record missing its id or checksum would be skipped silently by the main
# loop, which is the same class of bug as the unmatched --item fail-open.
# ---------------------------------------------------------------------------

items=$("$FETCH" list 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')
if [ "$items" -gt 0 ]; then
    ok "manifest parses ($items items)"
else
    bad "manifest parses" "list produced no items"
fi

malformed=$(awk '
    /^[ \t]*\[\[item\]\][ \t]*$/ { n++; id[n]=""; kind[n]=""; sha[n]=""; dest[n]=""; next }
    n == 0 { next }
    /^[ \t]*id[ \t]*=/     { id[n]   = $0 }
    /^[ \t]*kind[ \t]*=/   { kind[n] = $0 }
    /^[ \t]*sha256[ \t]*=/ { sha[n]  = $0 }
    /^[ \t]*dest[ \t]*=/   { dest[n] = $0 }
    END {
        for (i = 1; i <= n; i++)
            if (id[i] == "" || kind[i] == "" || sha[i] == "" || dest[i] == "")
                print "item " i
    }
' "$SCRIPT_DIR/manifest.toml")

if [ -z "$malformed" ]; then
    ok "every manifest item has id, kind, dest and sha256"
else
    bad "every manifest item has id, kind, dest and sha256" "$malformed"
fi

# ---------------------------------------------------------------------------
# 3. Structural invariants of the record set
#
# These are the ones a silent parse cannot catch. A `patch` naming a file that
# is not there fails only at fetch time, on the machine least able to diagnose
# it; a divergence recorded without a patch is an undisclosed modification of
# third-party material; a malformed `rename` rule would restructure a tree into
# the wrong shape and only surface as a checksum mismatch with no clue why.
# ---------------------------------------------------------------------------

MANIFEST="$SCRIPT_DIR/manifest.toml"

structural=$(awk -v dir="$SCRIPT_DIR" '
    function fault(msg) { print "item " (id == "" ? "#" n : id) ": " msg }
    function check() {
        if (n == 0) return
        if (patch != "" && !seen_patch_file) fault("patch file not found: " patch)
        if (upstream != "" && patch == "") fault("upstream_sha256 without a patch — undisclosed divergence")
        if (patch != "" && upstream == "") fault("patch without upstream_sha256 — the pristine hash is the disclosure")
        if (base != "" && base != "repo" && base != "references") fault("unknown base: " base)
        if (subdir != "" && kind != "git") fault("subdir on a non-git item")
        if (rename != "" && kind != "git") fault("rename on a non-git item")
        if (rename != "") {
            cnt = split(rename, rules, "|")
            for (r = 1; r <= cnt; r++) {
                if (rules[r] !~ /=/)          fault("rename rule has no \"=\": " rules[r])
                else {
                    from = rules[r]; sub(/=.*/, "", from)
                    to   = rules[r]; sub(/^[^=]*=/, "", to)
                    if (from !~ /\/$/) fault("rename source must end in \"/\": " from)
                    if (to   !~ /\/$/) fault("rename target must end in \"/\": " to)
                }
            }
        }
    }
    /^[ \t]*\[\[item\]\][ \t]*$/ {
        check(); n++
        id=""; kind=""; patch=""; upstream=""; base=""; subdir=""; rename=""
        seen_patch_file=0
        next
    }
    n == 0 { next }
    {
        # Same subset the fetch.sh parser reads, minus the columns this check
        # does not use. Kept structurally identical on purpose: a check that
        # parses differently from the thing it checks proves nothing.
        line = $0
        gsub(/^[ \t]+|[ \t]+$/, "", line)
        if (line ~ /^#/) next
        eq = index(line, "=")
        if (eq == 0) next
        key = substr(line, 1, eq - 1); gsub(/[ \t]+$/, "", key)
        val = substr(line, eq + 1);    gsub(/^[ \t]+|[ \t]+$/, "", val)
        if (substr(val, 1, 1) == "[") {
            gsub(/^\[|\]$/, "", val)
            cnt = split(val, parts, ",")
            val = ""
            for (i = 1; i <= cnt; i++) {
                p = parts[i]
                gsub(/^[ \t]*"|"[ \t]*$/, "", p)
                if (p == "") continue
                val = (val == "") ? p : val "|" p
            }
        } else {
            gsub(/^"|"$/, "", val)
        }
        if (key == "id")             id = val
        else if (key == "kind")      kind = val
        else if (key == "base")      base = val
        else if (key == "subdir")    subdir = val
        else if (key == "rename")    rename = val
        else if (key == "upstream_sha256") upstream = val
        else if (key == "patch") {
            patch = val
            seen_patch_file = ((getline junk < (dir "/" val)) >= 0)
            close(dir "/" val)
        }
    }
    END { check() }
' "$MANIFEST")

if [ -z "$structural" ]; then
    ok "patches exist, divergences are disclosed, rename rules are well formed"
else
    bad "patches exist, divergences are disclosed, rename rules are well formed" "$structural"
fi

# ---------------------------------------------------------------------------
# 4. OS-D2 decisions are still reflected in the manifest
#
# Decision 1: `libraries/standard/` is reconstructed rather than vendored, so
# the item must be present AND carry the whole recipe — losing `subdir`,
# `rename` or `patch` would still parse and still fetch *something*, just not
# the tree the runtime loads.
#
# Decision 5: `sysml-specification.pdf` is out of the fetch set. It cannot be
# pinned (OMG re-renders it in place), so its return would be a permanently
# red item.
# ---------------------------------------------------------------------------

stdlib_record=$(awk '
    /^[ \t]*\[\[item\]\][ \t]*$/ { inrec = 0 }
    /^[ \t]*id[ \t]*=[ \t]*"standard-library"[ \t]*$/ { inrec = 1 }
    inrec { print }
' "$MANIFEST")

missing=''
if [ -z "$stdlib_record" ]; then
    missing='the whole record'
else
    for key in base subdir rename patch upstream_sha256 sha256 commit; do
        printf '%s\n' "$stdlib_record" | grep -q "^[ \t]*$key[ \t]*=" || missing="$missing $key"
    done
    # The reconstruction is only sound if it reads the SAME commit the pilot
    # tree is pinned at; two pins drifting apart is a silent inconsistency.
    stdlib_commit=$(printf '%s\n' "$stdlib_record" | awk -F'"' '/^[ \t]*commit[ \t]*=/ { print $2; exit }')
    pilot_commit=$(awk -F'"' '/^[ \t]*id[ \t]*=[ \t]*"pilot-implementation"/ { f = 1 }
                              f && /^[ \t]*commit[ \t]*=/ { print $2; exit }' "$MANIFEST")
    [ "$stdlib_commit" = "$pilot_commit" ] || missing="$missing commit-agrees-with-pilot-implementation"
fi

if [ -z "$missing" ]; then
    ok "standard-library item carries the full reconstruction recipe (OS-D2 #1)"
else
    bad "standard-library item carries the full reconstruction recipe (OS-D2 #1)" "missing:$missing"
fi

if grep -q '^[ \t]*dest[ \t]*=.*sysml-specification\.pdf' "$MANIFEST" \
   || grep -q '^[ \t]*id[ \t]*=[ \t]*"api-services-spec-pdf"' "$MANIFEST"; then
    bad "sysml-specification.pdf stays out of the fetch set (OS-D2 #5)" \
        "an item for it is back in the manifest"
else
    ok "sysml-specification.pdf stays out of the fetch set (OS-D2 #5)"
fi

# ---------------------------------------------------------------------------
# 5. Directory hashing agrees with the Rust provenance gate
#
# spec-drop.toml records the aggregate hash of the API-Services metamodel
# directory, computed by sysml-spec-tests. Reproducing it here proves the
# shell implementation of the aggregate algorithm matches the Rust one. Skips
# rather than fails when the tree is absent, so the check is usable on a
# fresh clone before `fetch` has run.
# ---------------------------------------------------------------------------

metamodel="$REPO_ROOT/references/sysmlv2/SysML-v2-API-Services/conf/json/schema/metamodel"
expected=$(awk '/^sha256 = / && prev ~ /metamodel/ { gsub(/^sha256 = "|"$/, "", $0); print; exit }
                { prev = $0 ~ /^path = / ? $0 : prev }' \
           "$REPO_ROOT/references/sysmlv2/spec-drop.toml" 2>/dev/null || true)

if [ ! -d "$metamodel" ]; then
    printf 'skip %s\n' "aggregate hash matches spec-drop.toml (tree not fetched)"
elif [ -z "$expected" ]; then
    bad "aggregate hash matches spec-drop.toml" "could not read the expected value"
else
    actual=$("$FETCH" hash "$metamodel" | cut -d' ' -f1)
    if [ "$actual" = "$expected" ]; then
        ok "aggregate hash matches spec-drop.toml"
    else
        bad "aggregate hash matches spec-drop.toml" "expected $expected, got $actual"
    fi
fi

# ---------------------------------------------------------------------------
# 4. REGRESSION: an unmatched --item must fail, not report an empty success
#
# This previously exited 0 with "all 0 items verified", which in CI is a
# silently-green no-op that only surfaces downstream as an unrelated-looking
# build failure.
# ---------------------------------------------------------------------------

out=$("$FETCH" verify --item definitely-not-an-item 2>&1) && status=0 || status=$?

if [ "$status" -eq 0 ]; then
    bad "unmatched --item fails" "exited 0; output: $out"
elif printf '%s' "$out" | grep -q "all 0 items verified"; then
    bad "unmatched --item fails" "reported an empty success"
else
    ok "unmatched --item fails (exit $status)"
fi

if printf '%s' "$out" | grep -q "known items:"; then
    ok "unmatched --item lists the known item names"
else
    bad "unmatched --item lists the known item names" "$out"
fi

# A real item must still be accepted, so the guard is not simply rejecting
# everything.
first_id=$("$FETCH" list 2>/dev/null | tail -n +2 | head -1 | cut -d' ' -f1)
if "$FETCH" verify --item "$first_id" >/dev/null 2>&1; then
    ok "a valid --item is still accepted ($first_id)"
else
    # Verify can legitimately fail when the tree is absent; what must not
    # happen is rejection as an unknown item.
    if "$FETCH" verify --item "$first_id" 2>&1 | grep -q "no manifest item matches"; then
        bad "a valid --item is still accepted" "$first_id was rejected as unknown"
    else
        printf 'skip %s\n' "a valid --item is still accepted (tree not fetched)"
    fi
fi

# Glob selection must work, since the CI workflows rely on it.
if "$FETCH" verify --item 'kpar-*' >/dev/null 2>&1 \
   || ! "$FETCH" verify --item 'kpar-*' 2>&1 | grep -q "no manifest item matches"; then
    ok "glob --item selection matches"
else
    bad "glob --item selection matches" "kpar-* matched nothing"
fi

# ---------------------------------------------------------------------------

printf '\n%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
