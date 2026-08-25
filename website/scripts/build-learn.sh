#!/usr/bin/env bash
# Build the SysML v2 Book at the revision pinned in content-lock.json and
# stage it under public/learn/, so the portal serves it at /sysml-rs/learn/.
#
# The Book stays a separate canonical repository; this script never builds an
# unpinned revision. Update the pin in content-lock.json deliberately.
set -euo pipefail

WEBSITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK_FILE="${WEBSITE_DIR}/content-lock.json"
SRC_DIR="${WEBSITE_DIR}/.learn-src"
OUT_DIR="${WEBSITE_DIR}/public/learn"

BOOK_REPO="$(node -e "console.log(require('${LOCK_FILE}').book.repository)")"
BOOK_COMMIT="$(node -e "const c = require('${LOCK_FILE}').book.commit; if (!c) { process.exit(3); } console.log(c)")" || {
  echo "content-lock.json has no book.commit pin; refusing to build /learn/." >&2
  exit 1
}

if [ ! -d "${SRC_DIR}/.git" ]; then
  git clone --no-checkout "${BOOK_REPO}" "${SRC_DIR}"
fi
git -C "${SRC_DIR}" fetch origin "${BOOK_COMMIT}"
git -C "${SRC_DIR}" checkout --detach --force "${BOOK_COMMIT}"

ACTUAL="$(git -C "${SRC_DIR}" rev-parse HEAD)"
if [ "${ACTUAL}" != "${BOOK_COMMIT}" ]; then
  echo "Pinned Book commit mismatch: wanted ${BOOK_COMMIT}, got ${ACTUAL}" >&2
  exit 1
fi

rm -rf "${OUT_DIR}"
# site-url is overridden for the portal base path; the standalone Book keeps
# its own site-url from book.toml.
MDBOOK_OUTPUT__HTML__SITE_URL="/sysml-rs/learn/" \
  mdbook build "${SRC_DIR}" --dest-dir "${OUT_DIR}"

echo "Built Book ${BOOK_COMMIT} into ${OUT_DIR}"
