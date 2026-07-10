petal::route_file!(spec: petal::account_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    let owner = match crate::infra_parts::host_calls::wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match crate::infra_parts::credentials::load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    match crate::infra_parts::clob_l2::clob_l2_get_json(owner, &creds, "/data/orders", &[]) {
        Ok(orders) => petal::read_json_value(&orders),
        Err(resp) => resp,
    }
});
