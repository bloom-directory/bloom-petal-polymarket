crate::route_file!(spec: crate::store_dir_spec(), ctx_list: |ctx: &crate::Ctx| {
    let wallet = crate::param(ctx, "wallet")?;
    let mut out = vec![crate::writable("new")];
    out.extend(crate::services::fund::session_ids(wallet));
    Ok(out)
});
