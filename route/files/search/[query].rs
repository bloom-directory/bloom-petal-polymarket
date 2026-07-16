petal::route_file!(spec: petal::http_read_spec(30_000), read: |ctx: &petal::Ctx| {
    let query = match petal::param(ctx, "query") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let query = query.replace('+', " ");
    match crate::infra_parts::http::get_json::<serde_json::Value>(&crate::infra_parts::util::url_with_query(
        &format!("{}/public-search", crate::runtime_config::gamma_url()),
        &[("q", &query)],
    )) {
        Ok(value) => petal::read_json_value(&value),
        Err(resp) => resp,
    }
});
