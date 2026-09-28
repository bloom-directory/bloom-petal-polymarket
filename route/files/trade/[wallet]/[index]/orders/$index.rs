petal::route_file!(spec: petal::store_dir_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]), ctx_list: |ctx: &petal::Ctx| {
    let account = crate::account::number(ctx)?;
    let wallet = petal::wallet_param(ctx)?;
    Ok(crate::trade_flow_parts::posting::discoverable_order_ids(wallet, account)
        .unwrap_or_default()
        .into_iter()
        .map(|id| petal::dir(&id))
        .collect())
});
