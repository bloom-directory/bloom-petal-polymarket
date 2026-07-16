petal::route_file!(spec: petal::http_read_spec(5_000).caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
    let slug = match petal::param(ctx, "slug") { Ok(value) => value, Err(resp) => return resp };
    crate::relayer_actions::redeem_plan(wallet, slug)
});
