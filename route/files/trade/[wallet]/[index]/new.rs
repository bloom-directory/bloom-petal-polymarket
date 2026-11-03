petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store"]), read:
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"Write JSON to create an unsigned draft under drafts/; nothing is signed or posted until the owner approves it.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"Yes","side":"buy","amount":"5","limit_price":"0.01","order_type":"GTC"}
slug, outcome (Yes or No): from ../../../markets/<slug>/market.json.
side: buy (default) or sell.
amount: pUSD of shares to buy, before fees; shares to sell when selling, in steps of 0.01. A buy can also pay up to the draft's fee_cap in fees, so for a total budget set amount below it. pUSD is the trading collateral (see account buying_power.json); USDC.e is only for funding.
Numbers are JSON strings: positive decimals with at most 6 decimal places. Prices are pUSD per share, on the market's tick, between tick and 1 - tick.
Price: limit_price fixes the order price. Without it, max_price (buy) or min_price (sell) picks a price from the live book within that bound; the draft is refused when the book is outside it. If both are given, limit_price must be within the bound.
order_type sets lifetime, separately from price: GTC fills what it can now and leaves the rest on the book; FAK fills what it can now and cancels the rest; FOK fills all now or cancels. Default is GTC with limit_price, FAK without. Orders are never post-only, so a GTC that crosses the book fills immediately. GTD is not supported.
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::draft::create_trade_draft(wallet, body)
    }
);
