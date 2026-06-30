crate::route_file!(spec: crate::http_read_spec(30_000), read: |ctx: &crate::Ctx| {
    let slug = match crate::param(ctx, "slug") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    match crate::market_by_slug(slug) {
        Ok(market) => crate::read_json_value(&market),
        Err(resp) => resp,
    }
});
