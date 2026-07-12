petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read", "bloom:vfs.write"]), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write {\"revalidate\":true} to revalidate this draft and stage the final review artifact. Revalidated drafts can then be posted by writing {\"post\":true} to post; resting GTC orders can be cancelled from their receipt.\n".to_vec())
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
        crate::trade_flow_parts::revalidate::revalidate_trade_draft(wallet, id, body)
    }
);
