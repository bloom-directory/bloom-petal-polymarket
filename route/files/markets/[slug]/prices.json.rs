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
    let midpoint = match crate::infra_parts::http::get_json::<serde_json::Value>(&crate::infra_parts::util::url_with_query(
        &format!("{}{}", crate::constants::CLOB, "/midpoint"),
        &[("token_id", token_id)],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let spread = match crate::infra_parts::http::get_json::<serde_json::Value>(&crate::infra_parts::util::url_with_query(
        &format!("{}{}", crate::constants::CLOB, "/spread"),
        &[("token_id", token_id)],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let best_buy = match crate::infra_parts::http::get_json::<serde_json::Value>(&crate::infra_parts::util::url_with_query(
        &format!("{}{}", crate::constants::CLOB, "/price"),
        &[("token_id", token_id), ("side", "BUY")],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    petal::read_json_value(&serde_json::json!({
        "token_id": token_id,
        "midpoint": midpoint,
        "spread": spread,
        "best_buy": best_buy,
    }))
});
