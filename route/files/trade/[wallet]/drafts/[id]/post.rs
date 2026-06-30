crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| {
        crate::DispatchResponse::Read(b"write {\"post\":true} to sign and post a revalidated draft, then write a private receipt. This performs a value-moving CLOB POST /order.\n".to_vec())
    },
    write: |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match crate::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::post_trade_draft(wallet, id, body)
    }
);
