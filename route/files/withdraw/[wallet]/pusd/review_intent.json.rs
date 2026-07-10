petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    match petal::param(ctx, "wallet") { Ok(wallet) => petal::read_store(&format!("actions/{wallet}/withdraw-pusd/review_intent.json"), 1_048_576), Err(resp) => resp }
});
