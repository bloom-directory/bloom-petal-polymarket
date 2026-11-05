petal::route_file!(spec: petal::store_read_spec().caps(&["bloom:store", "bloom:sign.status"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(w) => w, Err(r) => return r };
    let batch = match crate::polymarket::relayer_batch::current_scope(&mut petal::signing::CtxHost(ctx), &format!("onboard/{wallet}")) { Ok(s) => s, Err(r) => return r };
    petal::signing::approval_read(ctx, wallet, &[&format!("onboard/{wallet}/login"), batch.as_deref().unwrap_or("onboard/no-batch")])
});
