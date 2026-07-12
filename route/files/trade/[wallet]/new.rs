petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]), read:
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"write JSON to create a reviewable draft, e.g.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","amount":"1","max_price":"0.01"}
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::draft::create_trade_draft(wallet, body)
    }
);
