crate::bloom_read_component!("markets/[slug]/market.json", |ctx: &crate::Ctx| {
    let Some(slug) = crate::route_param_or_segment(ctx, "slug", 1) else {
        return crate::route_invalid("missing slug");
    };
    crate::read_market(slug, "market.json")
});
