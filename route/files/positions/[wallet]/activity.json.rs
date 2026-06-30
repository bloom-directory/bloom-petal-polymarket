crate::route_file!(spec: crate::wallet_http_read_spec(10_000), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::position_activity_json(wallet)
});
