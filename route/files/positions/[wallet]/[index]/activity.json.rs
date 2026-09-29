petal::route_file!(spec: petal::wallet_http_read_spec(10_000), read: |ctx: &petal::Ctx| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    let wallet = match petal::wallet_param(ctx) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let user = match crate::public_reads::position_user(wallet, account) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match crate::infra_parts::http::get_json::<serde_json::Value>(&crate::infra_parts::util::url_with_query(
        &format!("{}/activity", crate::runtime_config::data_url()),
        &[("user", &user)],
    )) {
        Ok(value) => petal::read_json_value(&value),
        Err(resp) => resp,
    }
});
