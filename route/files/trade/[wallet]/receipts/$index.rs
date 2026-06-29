crate::bloom_ctx_dir_component!("trade/[wallet]/receipts/$index", |ctx: &crate::Ctx| {
    let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
        return Err(crate::route_invalid("missing wallet"));
    };
    Ok(crate::store_ids(
        &format!("trade/{wallet}/receipts/"),
        "/receipt.json",
    ))
});
