crate::route_file!(spec: crate::static_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    crate::services::onboard::plan_md(wallet)
});
