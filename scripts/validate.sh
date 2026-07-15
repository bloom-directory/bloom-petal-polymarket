#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BLOOM_REPO="${BLOOM_REPO:-}"

"$ROOT/scripts/build.sh"

if [ -n "$BLOOM_REPO" ]; then
  cargo run --manifest-path "$BLOOM_REPO/Cargo.toml" -p bloom -- petal build "$ROOT"
elif command -v bloom >/dev/null 2>&1; then
  bloom petal build "$ROOT"
else
  echo "set BLOOM_REPO=/path/to/bloom or install bloom to validate the package" >&2
  exit 127
fi
