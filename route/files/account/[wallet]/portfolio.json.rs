crate::bloom_read_component!(crate::account_read_spec(), |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_account(wallet, "portfolio.json")
});
