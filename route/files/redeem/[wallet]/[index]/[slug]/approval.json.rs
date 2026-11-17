petal::route_file!(spec: petal::store_read_spec().caps(&["bloom:store", "bloom:sign.status"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(w) => w, Err(r) => return r };
    let slug = match petal::param(ctx, "slug") { Ok(s) => s, Err(r) => return r };
    let scope = match crate::polymarket::relayer_batch::current_scope(&mut petal::signing::CtxHost(ctx), &format!("actions/{wallet}/redeem/{slug}")) { Ok(Some(s)) => s, Ok(None) => return petal::error(-3, "no pending signing approval"), Err(r) => return r };
    petal::signing::approval_read(ctx, wallet, &[&scope])
});
