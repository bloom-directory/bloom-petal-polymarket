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
    let midpoint = match crate::get_json::<serde_json::Value>(&crate::url_with_query(
        &format!("{}{}", crate::CLOB, "/midpoint"),
        &[("token_id", token_id)],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let spread = match crate::get_json::<serde_json::Value>(&crate::url_with_query(
        &format!("{}{}", crate::CLOB, "/spread"),
        &[("token_id", token_id)],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let best_buy = match crate::get_json::<serde_json::Value>(&crate::url_with_query(
        &format!("{}{}", crate::CLOB, "/price"),
        &[("token_id", token_id), ("side", "BUY")],
    )) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_json_value(&serde_json::json!({
        "token_id": token_id,
        "midpoint": midpoint,
        "spread": spread,
        "best_buy": best_buy,
    }))
});
