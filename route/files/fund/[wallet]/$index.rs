petal::route_file!(spec: petal::store_dir_spec(), ctx_list: |ctx: &petal::Ctx| {
    let wallet = petal::param(ctx, "wallet")?;
    let mut out = vec![petal::writable("new")];
    out.extend(petal::dirs(crate::infra_parts::lists::store_ids(
        &format!("fund/{wallet}/requests/"),
        ".json",
    )));
    Ok(out)
});
