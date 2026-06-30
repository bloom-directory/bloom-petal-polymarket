crate::route_file!(spec: crate::http_read_spec(30_000), read: |ctx: &crate::Ctx| {
    let query = match crate::param(ctx, "query") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let query = query.replace('+', " ");
    match crate::get_json::<serde_json::Value>(&crate::url_with_query(
        &format!("{}{}", crate::GAMMA, "/public-search"),
        &[("q", &query)],
    )) {
        Ok(value) => crate::read_json_value(&value),
        Err(resp) => resp,
    }
});
