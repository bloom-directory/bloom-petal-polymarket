petal::route_file!(spec: petal::account_read_spec(), read: |ctx: &petal::Ctx| {
    match petal::param(ctx, "wallet") {
        Ok(wallet) => crate::account_views::funding_options(wallet),
        Err(resp) => resp,
    }
});
