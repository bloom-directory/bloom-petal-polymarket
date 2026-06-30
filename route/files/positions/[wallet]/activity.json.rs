crate::route_file!(spec: crate::wallet_http_read_spec(10_000), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let user = match crate::position_user(wallet) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match crate::get_json::<serde_json::Value>(&crate::url_with_query(
        &format!("{}{}", crate::DATA, "/activity"),
        &[("user", &user)],
    )) {
        Ok(value) => crate::read_json_value(&value),
        Err(resp) => resp,
    }
});
