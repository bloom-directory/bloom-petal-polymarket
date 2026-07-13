petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
    match petal::param(ctx, "wallet") {
        Ok(wallet) => crate::account_views::buying_power(wallet),
        Err(resp) => resp,
    }
});
