petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
    let id = match petal::param(ctx, "id") { Ok(value) => value, Err(resp) => return resp };
    petal::read_store(&format!("trade/{wallet}/drafts/{id}/approval.json"), 1_048_576)
});
