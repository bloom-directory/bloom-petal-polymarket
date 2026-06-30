crate::route_file!(spec: crate::http_read_spec(30_000), read: |ctx: &crate::Ctx| {
    let query = match crate::param(ctx, "query") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::search_results(query)
});
