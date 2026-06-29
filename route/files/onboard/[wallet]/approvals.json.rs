crate::bloom_read_component!("onboard/[wallet]/approvals.json", |ctx: &crate::Ctx| {
    let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
        return crate::route_invalid("missing wallet");
    };
    crate::read_onboard(wallet, "approvals.json")
});
