petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write {\"cancel\":true} to cancel the posted CLOB order recorded by this receipt. Cancelling uses CLOB DELETE /order and updates the private receipt/draft status.\n".to_vec())
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
        crate::trade_flow_parts::posting::cancel_trade_receipt(wallet, id, body)
    }
);
