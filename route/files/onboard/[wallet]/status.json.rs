crate::route_file!(spec: crate::chain_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::read_onboard(wallet, "status.json")
});
