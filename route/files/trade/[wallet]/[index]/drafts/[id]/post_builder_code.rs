// Builder-coded posting. Signs under the fee-bearing operation class with
// the builder fee declared in the approval claim.
//
// Why a second route at all: Broker requires an operation class to be
// uniformly fee-bearing or uniformly fee-free. A class catalogued with a fee
// asset refuses a claim declaring no fee (`FEE_REQUIRED`), and a class
// without one refuses a claim that declares a fee (`FEE_NOT_ALLOWED`). So the
// fee-bearing variant cannot share `polymarket.order.poly1271` with plain
// orders; it needs its own class, reached through its own route. Hyperliquid
// made the same structural split, but on a different fee model: it names a
// builder by address and takes the fee per order within a cap the user
// approved on-chain. Polymarket names a builder by an opaque `bytes32` code
// whose maker and taker rates live on the builder's profile, with no per-user
// approval and no per-order fee field, so the class and route say "builder
// code" rather than borrowing Hyperliquid's "builder order".
//
// `polymarket.builder_code_order.poly1271` is catalogued with the polygon/pusd fee
// asset by Bloom's `FEE_BEARING_OPERATION_CLASSES`
// (`crates/bloom/src/triad_enrollment.rs`). That name and asset pair must
// keep matching this route's class and the fee this Petal eventually
// declares, or signing answers `FEE_ASSET_MISMATCH`.
//
petal::route_file!(
    spec: petal::signing_write_spec("polymarket.builder_code_order.poly1271")
        .caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:chain", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(
            b"write {\"post\":true} to sign and post a revalidated draft under the fee-bearing polymarket.builder_code_order.poly1271 class. The claim declares the builder fee as a ceiling from settings/builder-fee-ceiling, and the ceremony states the builder, the builder fee and the total it covers, and that Polymarket's own fee sits outside that total. Requires a configured builder code and ceiling; plain orders post through `post`. This performs a value-moving CLOB POST /order.\n"
                .to_vec(),
        )
    },
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let account = match crate::account::number(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match petal::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::posting::post_builder_code_trade_draft(
            ctx, wallet, account, id, body,
        )
    }
);
