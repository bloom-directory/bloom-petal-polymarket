petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    use crate::prelude::*;

    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    let (owner, status) = match crate::account_views::wallet_status(wallet, account) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let balance_allowance = match clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[
            ("asset_type", "COLLATERAL"),
            ("signature_type", if legacy_eoa { "0" } else { "3" }),
        ],
    ) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let has_balance = balance_allowance
        .get("balance")
        .and_then(parse_json_u256)
        .is_some_and(|balance| !balance.is_zero());
    let tradeable = status
        .get("tradeable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "spendable": {
            "asset": "pUSD",
            "raw": balance_allowance.get("balance").cloned().unwrap_or(serde_json::Value::Null),
            "source": "clob_balance_allowance",
            "clob_balance_allowance": balance_allowance,
        },
        "can_trade_now": !legacy_eoa && tradeable && has_balance,
        "credentials_read_only": legacy_eoa,
        "funding_needed": !has_balance,
        "funding_options_ref": format!("account/{wallet}/{account}/funding_options.json"),
    }))

});
