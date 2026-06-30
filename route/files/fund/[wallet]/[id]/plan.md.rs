crate::route_file!(spec: crate::store_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let id = match crate::param(ctx, "id") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let session = match crate::load_fund_session(wallet, id) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    crate::DispatchResponse::Read(crate::render_fund_plan(&session).into_bytes())
});
