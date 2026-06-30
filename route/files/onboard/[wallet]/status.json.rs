crate::route_file!(spec: crate::chain_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::validate_wallet_name(wallet) {
        return crate::error(-3, e.to_string());
    }
    let status = match crate::wallet_address(wallet) {
        Ok(owner) => match crate::local_status_for_wallet(wallet, owner) {
            Ok(status) => status,
            Err(resp) => return resp,
        },
        Err(_) => serde_json::json!({
            "wallet": wallet,
            "stage": "not_started",
            "running": false,
            "tradeable": false,
            "message": "write begin to mint or derive CLOB credentials"
        }),
    };
    crate::read_json_value(&status)
});
