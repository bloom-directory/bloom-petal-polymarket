petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let id = match petal::param(ctx, "id") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let session = match crate::fund_flow::load_fund_session(wallet, id) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    petal::read_json_value(&session)
});
