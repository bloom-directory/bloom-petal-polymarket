petal::route_file!(spec: petal::http_read_spec(2_000), read: |ctx: &petal::Ctx| {
    let slug = match petal::param(ctx, "slug") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let market = match crate::public_reads::market_by_slug(slug) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    let Some(token_id) = market.yes_token_id() else {
        return petal::error(-4, "market has no YES token id");
    };
    match crate::infra_parts::http::get_json::<crate::polymarket::OrderBook>(&crate::infra_parts::util::url_with_query(
        &format!("{}/book", crate::runtime_config::clob_url()),
        &[("token_id", token_id)],
    )) {
        Ok(book) => petal::read_json_value(&book),
        Err(resp) => resp,
    }
});
