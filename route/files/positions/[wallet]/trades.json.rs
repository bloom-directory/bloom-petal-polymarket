petal::route_file!(spec: petal::wallet_http_read_spec(10_000), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let user = match crate::public_reads::position_user(wallet) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match crate::infra_parts::http::get_json::<Vec<crate::polymarket::Trade>>(&crate::infra_parts::util::url_with_query(
        &format!("{}{}", crate::constants::DATA, "/trades"),
        &[("user", &user)],
    )) {
        Ok(value) => petal::read_json_value(&value),
        Err(resp) => resp,
    }
});
