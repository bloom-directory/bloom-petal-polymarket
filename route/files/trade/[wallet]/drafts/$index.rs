crate::route_file!(spec: crate::store_dir_spec(), ctx_list: |ctx: &crate::Ctx| {
    let wallet = crate::param(ctx, "wallet")?;
    Ok(crate::dirs(crate::store_ids(
        &format!("trade/{wallet}/drafts/"),
        "/order.json",
    )))
});
