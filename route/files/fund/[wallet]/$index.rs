crate::bloom_ctx_dir_component!("fund/[wallet]/$index", |ctx: &crate::Ctx| {
    let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
        return Err(crate::route_invalid("missing wallet"));
    };
    let mut out = vec!["new".to_string()];
    out.extend(crate::store_ids(
        &format!("fund/{wallet}/requests/"),
        ".json",
    ));
    Ok(out)
});
