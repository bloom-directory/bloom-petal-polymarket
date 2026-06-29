crate::bloom_ctx_dir_component!(|ctx: &crate::Ctx| {
    let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
        return Err(crate::route_invalid("missing wallet"));
    };
    let mut out = vec![crate::writable("new")];
    out.extend(crate::dirs(crate::store_ids(
        &format!("fund/{wallet}/requests/"),
        ".json",
    )));
    Ok(out)
});
