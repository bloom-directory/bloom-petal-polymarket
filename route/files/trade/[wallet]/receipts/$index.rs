crate::route_file!(spec: crate::store_dir_spec(), ctx_list: |ctx: &crate::Ctx| {
    let wallet = crate::param(ctx, "wallet")?;
    crate::services::trade::receipt_ids(wallet)
});
