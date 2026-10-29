petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store"]), read:
    // The complete request shape, so an agent never has to read this
    // package's source to draft an order.
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"Write JSON to create an unsigned draft under drafts/; nothing is signed or posted until the owner approves it.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"Yes","side":"buy","amount":"5","limit_price":"0.01","order_type":"GTC"}
slug, outcome: from ../../../markets/<slug>/market.json.
side: buy (default) or sell.
amount: USDC to spend when buying; shares to sell when selling.
limit_price: the order rests on the book at this price (GTC unless order_type says otherwise).
max_price (buy) or min_price (sell) instead of limit_price: a marketable order that fills immediately up to that bound and never rests (FAK). It is refused when the book is outside the bound.
order_type: GTC, FOK or FAK; GTD is not supported.
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::draft::create_trade_draft(wallet, body)
    }
);
