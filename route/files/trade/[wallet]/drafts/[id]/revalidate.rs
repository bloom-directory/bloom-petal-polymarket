crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| {
        crate::DispatchResponse::Read(b"write {\"revalidate\":true} to revalidate this draft and stage the final review artifact. Revalidated drafts can then be posted by writing {\"post\":true} to post; resting GTC orders can be cancelled from their receipt.\n".to_vec())
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
        crate::revalidate_trade_draft(wallet, id, body)
    }
);
