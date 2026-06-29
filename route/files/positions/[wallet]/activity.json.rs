crate::bloom_read_component!("positions/[wallet]/activity.json", |ctx: &crate::Ctx| {
    let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
        return crate::route_invalid("missing wallet");
    };
    crate::read_positions(wallet, "activity.json")
});
