petal::route_file!(spec: petal::account_read_spec(), read: |ctx: &petal::Ctx| {
    match crate::account::wallet_param(ctx) {
        Ok(wallet) => crate::account_views::builder_keys(wallet),
        Err(resp) => resp,
    }
});
