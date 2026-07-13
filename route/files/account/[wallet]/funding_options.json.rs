petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    match petal::param(ctx, "wallet") {
        Ok(wallet) => crate::account_views::funding_options(wallet),
        Err(resp) => resp,
    }
});
