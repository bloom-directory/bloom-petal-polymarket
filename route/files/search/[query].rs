crate::bloom_read_component!(|ctx: &crate::Ctx| {
    let Some(query) = crate::route_param_or_segment(ctx, "query", 1) else {
        return crate::route_invalid("missing query");
    };
    crate::read_search(query)
});
