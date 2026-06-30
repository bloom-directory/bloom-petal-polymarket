crate::bloom_ctx_dir_component!(crate::store_dir_spec(), |ctx: &crate::Ctx| {
    let wallet = crate::param(ctx, "wallet")?;
    let mut out = vec![crate::writable("new")];
    out.extend(crate::dirs(crate::store_ids(
        &format!("fund/{wallet}/requests/"),
        ".json",
    )));
    Ok(out)
});
