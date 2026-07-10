petal::route_file!(spec: petal::http_read_spec(30_000), read: |ctx: &petal::Ctx| {
    let slug = match petal::param(ctx, "slug") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    match crate::public_reads::market_by_slug(slug) {
        Ok(market) => petal::read_json_value(&market),
        Err(resp) => resp,
    }
});
