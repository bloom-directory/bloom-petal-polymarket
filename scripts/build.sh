#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="${POLYMARKET_APP_DIR:-$ROOT}"
ROUTE="$ROOT/route"
OUT="$APP/app/polymarket"
TMP="$ROOT/target/polymarket-v2"
CORE="$ROUTE/target/wasm32-unknown-unknown/release/bloom_polymarket_v2_route.wasm"
COMPONENT="$TMP/polymarket-route.wasm"

routes=(
  '$index'
  '$list'
  'meta/parity.json'
  'meta/route-contract.json'
  'markets/$index'
  'markets/$list'
  'markets/[slug]/$index'
  'markets/[slug]/$list'
  'markets/[slug]/market.json'
  'markets/[slug]/book.json'
  'markets/[slug]/prices.json'
  'search/$index'
  'search/$list'
  'search/[query]'
  'positions/$index'
  'positions/$list'
  'positions/[wallet]/$index'
  'positions/[wallet]/$list'
  'positions/[wallet]/positions.json'
  'positions/[wallet]/trades.json'
  'positions/[wallet]/activity.json'
  'onboard/$index'
  'onboard/$list'
  'onboard/[wallet]/$index'
  'onboard/[wallet]/$list'
  'onboard/[wallet]/begin'
  'onboard/[wallet]/status.json'
  'onboard/[wallet]/plan.md'
  'onboard/[wallet]/approvals.json'
  'onboard/[wallet]/review_intent.json'
  'onboard/[wallet]/approval.json'
  'account/$index'
  'account/$list'
  'account/[wallet]/$index'
  'account/[wallet]/$list'
  'account/[wallet]/portfolio.json'
  'account/[wallet]/orders.json'
  'account/[wallet]/status.json'
  'account/[wallet]/buying_power.json'
  'account/[wallet]/funding_options.json'
  'builder-keys/$index'
  'builder-keys/$list'
  'builder-keys/[wallet]/$index'
  'builder-keys/[wallet]/$list'
  'builder-keys/[wallet]/keys.json'
  'builder-keys/[wallet]/revoke'
  'settings/$index'
  'settings/$list'
  'settings/enso-api-key'
  'fund/$index'
  'fund/$list'
  'fund/[wallet]/$index'
  'fund/[wallet]/$list'
  'fund/[wallet]/new'
  'fund/[wallet]/[id]/$index'
  'fund/[wallet]/[id]/$list'
  'fund/[wallet]/[id]/plan.md'
  'fund/[wallet]/[id]/request.json'
  'fund/[wallet]/[id]/status.json'
  'fund/[wallet]/[id]/review_intent.json'
  'fund/[wallet]/[id]/approval.json'
  'fund/[wallet]/[id]/confirm'
  'redeem/$index'
  'redeem/$list'
  'redeem/[wallet]/$index'
  'redeem/[wallet]/$list'
  'redeem/[wallet]/[slug]/$index'
  'redeem/[wallet]/[slug]/$list'
  'redeem/[wallet]/[slug]/plan.md'
  'redeem/[wallet]/[slug]/review_intent.json'
  'redeem/[wallet]/[slug]/approval.json'
  'redeem/[wallet]/[slug]/confirm'
  'redeem/[wallet]/[slug]/receipt.json'
  'revoke-approvals/$index'
  'revoke-approvals/$list'
  'revoke-approvals/[wallet]/$index'
  'revoke-approvals/[wallet]/$list'
  'revoke-approvals/[wallet]/request/$index'
  'revoke-approvals/[wallet]/request/$list'
  'revoke-approvals/[wallet]/request/plan.md'
  'revoke-approvals/[wallet]/request/review_intent.json'
  'revoke-approvals/[wallet]/request/approval.json'
  'revoke-approvals/[wallet]/request/confirm'
  'revoke-approvals/[wallet]/request/receipt.json'
  'withdraw/$index'
  'withdraw/$list'
  'withdraw/[wallet]/$index'
  'withdraw/[wallet]/$list'
  'withdraw/[wallet]/pusd/$index'
  'withdraw/[wallet]/pusd/$list'
  'withdraw/[wallet]/pusd/plan.md'
  'withdraw/[wallet]/pusd/review_intent.json'
  'withdraw/[wallet]/pusd/approval.json'
  'withdraw/[wallet]/pusd/confirm'
  'withdraw/[wallet]/pusd/receipt.json'
  'trade/$index'
  'trade/$list'
  'trade/[wallet]/$index'
  'trade/[wallet]/$list'
  'trade/[wallet]/new'
  'trade/[wallet]/drafts/$index'
  'trade/[wallet]/drafts/$list'
  'trade/[wallet]/drafts/[id]/$index'
  'trade/[wallet]/drafts/[id]/$list'
  'trade/[wallet]/drafts/[id]/plan.md'
  'trade/[wallet]/drafts/[id]/order.json'
  'trade/[wallet]/drafts/[id]/approval.json'
  'trade/[wallet]/drafts/[id]/post_attempt.json'
  'trade/[wallet]/drafts/[id]/policy_check.json'
  'trade/[wallet]/drafts/[id]/quote.json'
  'trade/[wallet]/drafts/[id]/revalidate'
  'trade/[wallet]/drafts/[id]/review_intent.json'
  'trade/[wallet]/drafts/[id]/post'
  'trade/[wallet]/orders/$index'
  'trade/[wallet]/orders/$list'
  'trade/[wallet]/orders/[clob-order-id]/$index'
  'trade/[wallet]/orders/[clob-order-id]/$list'
  'trade/[wallet]/orders/[clob-order-id]/cancel'
  'trade/[wallet]/receipts/$index'
  'trade/[wallet]/receipts/$list'
  'trade/[wallet]/receipts/[id]/$index'
  'trade/[wallet]/receipts/[id]/$list'
  'trade/[wallet]/receipts/[id]/cancel'
  'trade/[wallet]/receipts/[id]/receipt.json'
  'obligations/$index'
  'obligations/$list'
  'obligations/[wallet].json'
)

command -v cargo >/dev/null 2>&1 || {
  echo "missing required tool: cargo" >&2
  exit 127
}
command -v wasm-tools >/dev/null 2>&1 || {
  echo "missing required tool: wasm-tools" >&2
  exit 127
}

cargo build \
  --manifest-path "$ROUTE/Cargo.toml" \
  --target wasm32-unknown-unknown \
  --release

mkdir -p "$TMP"
wasm-tools component new "$CORE" -o "$COMPONENT"
wasm-tools validate "$COMPONENT"

rm -rf "$OUT"
for route in "${routes[@]}"; do
  mkdir -p "$(dirname "$OUT/$route.wasm")"
  cp "$COMPONENT" "$OUT/$route.wasm"
  chmod 0644 "$OUT/$route.wasm"
done

echo "wrote ${#routes[@]} v2 route components under $OUT"
