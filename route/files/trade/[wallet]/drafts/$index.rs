petal::route_file!(spec: petal::store_dir_spec().caps(&["bloom:store"]), ctx_list: |ctx: &petal::Ctx| {
    let wallet = petal::wallet_param(ctx)?;
    Ok(petal::dirs(crate::infra_parts::lists::store_ids(
        &format!("trade/{wallet}/drafts/"),
        "/order.json",
    )))
});
