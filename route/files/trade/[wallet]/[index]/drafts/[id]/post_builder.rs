// Placement for builder-coded posting: the route and the fee-bearing
// operation class exist and are enrolled, but posting through it is refused
// until the builder fee is declared in the claim.
//
// Why a second route at all: Broker requires an operation class to be
// uniformly fee-bearing or uniformly fee-free. A class catalogued with a fee
// asset refuses a claim declaring no fee (`FEE_REQUIRED`), and a class
// without one refuses a claim that declares a fee (`FEE_NOT_ALLOWED`). So the
// fee-bearing variant cannot share `polymarket.order.poly1271` with plain
// orders; it needs its own class, reached through its own route. Hyperliquid
// made the same split (`hyperliquid.order` vs `hyperliquid.builder_order`).
//
// `polymarket.builder_order.poly1271` is catalogued with the polygon/pusd fee
// asset by Bloom's `FEE_BEARING_OPERATION_CLASSES`
// (`crates/bloom/src/triad_enrollment.rs`). That name and asset pair must
// keep matching this route's class and the fee this Petal eventually
// declares, or signing answers `FEE_ASSET_MISMATCH`.
//
// What is still missing, and why this refuses rather than signs: the fee
// amount. The signed `Order` struct carries no fee field, Polymarket keeps
// maker/taker rates in the builder profile that issued the code, and no
// configured rate or permitted endpoint exposes them to this Petal. A
// fee-bearing claim needs an exact amount, so declaring one today would mean
// inventing it. Until the rate has a source, plain orders keep posting
// through `post` under the fee-free class.
// Caps are only what this route imports today. The capability check refuses
// a route that declares an import it does not use, so `bloom:http`,
// `bloom:sign`, `bloom:chain` and `bloom:vfs.read` join this list when the
// signing path lands, matching `post`.
petal::route_file!(
    spec: petal::signing_write_spec("polymarket.builder_order.poly1271")
        .caps(&["bloom:store"]),
    read: |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(
            b"posts a revalidated draft under the fee-bearing polymarket.builder_order.poly1271 class, with the builder fee declared in the approval claim. Not yet available: the builder fee rate has no source this Petal can read, so the fee cannot be declared exactly. Plain orders post through `post`.\n"
                .to_vec(),
        )
    },
    write: |ctx: &petal::Ctx, _body: &[u8]| {
        // Resolve the same inputs the signing path will need, so a
        // misconfigured wallet, account or draft id still fails on its own
        // terms rather than behind the unimplemented-fee refusal.
        if let Err(resp) = crate::account::number(ctx) {
            return resp;
        }
        if let Err(resp) = petal::wallet_param(ctx) {
            return resp;
        }
        if let Err(resp) = petal::param(ctx, "id") {
            return resp;
        }
        match crate::account_views::resolve_builder_code() {
            Ok(Some(_)) => petal::error(
                -3,
                "builder-coded posting is not available yet: the builder fee rate has no source this Petal can read, so it cannot be declared in the approval claim. Post through `post` instead, which signs under the fee-free polymarket.order.poly1271 class",
            ),
            Ok(None) => petal::error(
                -3,
                "no builder code is configured; write one to settings/builder-code, or post through `post` which signs under the fee-free polymarket.order.poly1271 class",
            ),
            Err(resp) => resp,
        }
    }
);
