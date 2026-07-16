petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
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
    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    match crate::infra_parts::clob_l2::clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[
            ("asset_type", "COLLATERAL"),
            ("signature_type", if legacy_eoa { "0" } else { "3" }),
        ],
    ) {
        Ok(clob_balance_allowance) => {
            let status = match crate::onboarding::local_status_for_wallet(wallet, owner) {
                Ok(status) => status,
                Err(resp) => return resp,
            };
            petal::read_json_value(&serde_json::json!({
                "wallet": wallet,
                "owner": format!("{owner:#x}"),
                "credentials_present": true,
                "credentials_read_only": legacy_eoa,
                "clob_balance_allowance": clob_balance_allowance,
                "deposit_wallet": status
                    .get("deposit_wallet")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                "onboarding_state": {
                    "stage": status.get("stage").cloned().unwrap_or(serde_json::Value::Null),
                    "creds_present": status.get("creds_present").cloned().unwrap_or(serde_json::Value::Bool(true)),
                    "tradeable": if legacy_eoa { serde_json::Value::Bool(false) } else { status.get("tradeable").cloned().unwrap_or(serde_json::Value::Bool(false)) }
                }
            }))
        }
        Err(resp) => resp,
    }
});
