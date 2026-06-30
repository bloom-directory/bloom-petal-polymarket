crate::bloom_read_component!(crate::http_read_spec(30_000), |ctx: &crate::Ctx| {
    let slug = match crate::param(ctx, "slug") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_market(slug, "market.json")
});
