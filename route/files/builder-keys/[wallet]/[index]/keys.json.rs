petal::route_file!(spec: petal::account_read_spec(), read: |ctx: &petal::Ctx| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    match petal::wallet_param(ctx) {
        Ok(wallet) => crate::account_views::builder_keys(wallet, account),
        Err(resp) => resp,
    }
});
