petal::route_file!(spec: petal::write_spec(), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write {\"post\":true} to sign and post a revalidated draft, then write a private receipt. This performs a value-moving CLOB POST /order.\n".to_vec())
    },
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match petal::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::trade_flow_parts::posting::post_trade_draft(wallet, id, body)
    }
);
