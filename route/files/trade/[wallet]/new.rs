crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| crate::DispatchResponse::Read(br#"write JSON to create a reviewable draft, e.g.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","amount":"1","max_price":"0.01"}
"#.to_vec()),
    write: |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::create_trade_draft(wallet, body)
    }
);
