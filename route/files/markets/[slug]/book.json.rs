crate::route_file!(spec: crate::http_read_spec(2_000), read: |ctx: &crate::Ctx| {
    let slug = match crate::param(ctx, "slug") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let market = match crate::market_by_slug(slug) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    let Some(token_id) = market.yes_token_id() else {
        return crate::error(-4, "market has no YES token id");
    };
    match crate::get_json::<crate::polymarket::OrderBook>(&crate::url_with_query(
        &format!("{}{}", crate::CLOB, "/book"),
        &[("token_id", token_id)],
    )) {
        Ok(book) => crate::read_json_value(&book),
        Err(resp) => resp,
    }
});
