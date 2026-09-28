petal::route_file!(spec: petal::store_dir_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]), ctx_list: |ctx: &petal::Ctx| {
    let wallet = crate::account::wallet_param(ctx)?;
    Ok(crate::trade_flow_parts::posting::discoverable_order_ids(wallet)
        .unwrap_or_default()
        .into_iter()
        .map(|id| petal::dir(&id))
        .collect())
});
