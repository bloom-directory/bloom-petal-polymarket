crate::bloom_ctx_dir_component!(crate::store_dir_spec(), |ctx: &crate::Ctx| {
    let wallet = crate::param(ctx, "wallet")?;
    Ok(crate::dirs(crate::store_ids(
        &format!("trade/{wallet}/receipts/"),
        "/receipt.json",
    )))
});
