crate::bloom_read_component!(crate::wallet_http_read_spec(10_000), |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_positions(wallet, "activity.json")
});
