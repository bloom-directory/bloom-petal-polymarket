petal::route_file!(spec: petal::store_dir_spec(), ctx_list: |ctx: &petal::Ctx| {
    let wallet = petal::param(ctx, "wallet")?;
    Ok(petal::dirs(crate::infra_parts::lists::store_ids(
        &format!("trade/{wallet}/receipts/"),
        "/receipt.json",
    )))
});
