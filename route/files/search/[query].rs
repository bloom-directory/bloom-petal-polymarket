crate::bloom_read_component!(crate::http_read_spec(30_000), |ctx: &crate::Ctx| {
    let query = match crate::param(ctx, "query") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_search(query)
});
