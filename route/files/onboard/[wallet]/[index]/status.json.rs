petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    let wallet = match petal::wallet_param(ctx) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    let mut status = match crate::infra_parts::host_calls::wallet_address(wallet, account) {
        Ok(owner) => match crate::onboarding::local_status_for_wallet(wallet, owner) {
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
    status["approvals"]["preview_path"] = serde_json::json!(format!("onboard/{wallet}/{account}/approvals.json"));
    status["credential_action"] = serde_json::json!(format!("onboard/{wallet}/{account}/begin"));
    status["deposit_wallet_action"] = serde_json::json!(format!("onboard/{wallet}/{account}/continue"));
    if let Ok(bytes) = petal::sdk::store_get(&format!("onboard/{wallet}/next_action.json"), 4096)
        && let Ok(next) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        status["next_action"] = next["next_action"].clone();
    }
    petal::read_json_value(&status)
});
