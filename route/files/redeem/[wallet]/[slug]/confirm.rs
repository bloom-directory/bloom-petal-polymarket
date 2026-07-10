petal::route_file!(spec: petal::signing_write_spec("polymarket.relayer_batch"),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm to prepare or advance the exact redemption batch\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
        let slug = match petal::param(ctx, "slug") { Ok(value) => value, Err(resp) => return resp };
        crate::relayer_actions::confirm_redeem(wallet, slug, body)
    }
);
