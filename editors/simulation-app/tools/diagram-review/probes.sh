#!/bin/bash
# Interaction-probe runner — same one-invocation pattern as run.sh.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$HERE/../.."
REPO="$APP/../.."
OUT="${1:-$APP/test-results/diagram-review}"
mkdir -p "$OUT"

"$REPO/target/release/sysml-api" 127.0.0.1:8080 > "$OUT/api-probe.log" 2>&1 &
API_PID=$!
(cd "$APP" && ./node_modules/.bin/vite --port 3010 --strictPort > "$OUT/vite-probe.log" 2>&1) &
VITE_PID=$!

for i in $(seq 1 30); do
  curl -s -m 1 -o /dev/null http://127.0.0.1:3010/ && break
  sleep 1
done

node "$HERE/probe.mjs" "$OUT"
STATUS=$?
kill $API_PID $VITE_PID 2>/dev/null
exit $STATUS
