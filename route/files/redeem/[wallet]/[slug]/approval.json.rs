petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
    let slug = match petal::param(ctx, "slug") { Ok(value) => value, Err(resp) => return resp };
    petal::read_store(&format!("actions/{wallet}/redeem/{slug}/approval.json"), 1_048_576)
});
