petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    match petal::wallet_param(ctx) {
        Ok(wallet) => crate::account_views::funding_options(wallet),
        Err(resp) => resp,
    }
});
