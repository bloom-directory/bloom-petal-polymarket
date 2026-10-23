#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PETAL_REV="2beed2ff344ce2b0c112e07096027e1ae0404007"

# A release build embeds this release's default builder code. The release
# workflow passes it as the generic PETAL_COMPILE_TIME_SECRET; re-export it
# under the name route/src/builder_code.rs reads. The code is public on-chain
# data, not a credential: it is a build input only because this repository is
# public, and it is bound into the approval the owner signs either way. A
# build that sets nothing has no default, and orders carry a zero builder.
if [[ "${PETAL_COMPILE_TIME_SECRET+x}" == "x" ]]; then
  if [[ -z "$PETAL_COMPILE_TIME_SECRET" ]]; then
    echo "PETAL_COMPILE_TIME_SECRET is set but empty" >&2
    exit 1
  fi
  export POLYMARKET_BUILDER_CODE="$PETAL_COMPILE_TIME_SECRET"
  unset PETAL_COMPILE_TIME_SECRET
fi

if [[ -n "${PETAL_BIN:-}" ]]; then
  "$PETAL_BIN" build --root "$ROOT"
else
  tool_root="$ROOT/target/petal-tool"
  cargo install \
    --git https://github.com/bloom-directory/petal \
    --rev "$PETAL_REV" \
    --locked \
    --root "$tool_root" \
    bloom-petal-cli
  "$tool_root/bin/petal" build --root "$ROOT"
fi
