petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    match crate::account::wallet_param(ctx) { Ok(wallet) => petal::read_store(&format!("actions/{wallet}/revoke-approvals/review_intent.json"), 1_048_576), Err(resp) => resp }
});
