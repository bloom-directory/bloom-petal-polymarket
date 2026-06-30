crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| {
        crate::DispatchResponse::Read(b"write {\"cancel\":true} to cancel the posted CLOB order recorded by this receipt. Cancelling uses CLOB DELETE /order and updates the private receipt/draft status.\n".to_vec())
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
        crate::cancel_trade_receipt(wallet, id, body)
    }
);
