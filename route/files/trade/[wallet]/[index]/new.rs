petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store"]), read:
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"write JSON to create a reviewable draft (Polymarket settles on Polygon). Fields:
  slug, outcome   market slug and outcome name
  side            "buy" (default) or "sell"
  amount          buy: USDC to spend; sell: shares to sell
  limit_price     resting order at this price (0-1); order_type defaults to GTC
  max_price       buy without limit_price: marketable order at the best ask, refused if the ask is above this cap
  min_price       sell without limit_price: marketable order at the best bid, refused if the bid is below this floor
  order_type      GTC, FOK or FAK; defaults to GTC with limit_price, else FAK
Resting limit buy:
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","side":"buy","amount":"5","limit_price":"0.01","order_type":"GTC"}
Marketable buy:
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","side":"buy","amount":"1","max_price":"0.5"}
On success the draft appears as drafts/<id>/ (plan.md, quote.json); nothing is signed or posted until drafts/<id>/revalidate and post.
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::draft::create_trade_draft(wallet, body)
    }
);
