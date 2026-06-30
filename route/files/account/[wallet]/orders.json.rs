crate::route_file!(spec: crate::account_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::validate_wallet_name(wallet) {
        return crate::error(-3, e.to_string());
    }
    let owner = match crate::wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match crate::load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    match crate::clob_l2_get_json(owner, &creds, "/data/orders", &[]) {
        Ok(orders) => crate::read_json_value(&orders),
        Err(resp) => resp,
    }
});
