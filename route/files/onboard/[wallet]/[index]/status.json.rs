petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read", "bloom:sign.status"]), read: |ctx: &petal::Ctx| {
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
    let approval_pending = match crate::approval::approval_projection(ctx, wallet, &crate::onboarding::approval_key(wallet), &["polymarket.clob_auth.v1", "polymarket.onboard"]) {
        Ok(value) => value.is_some(),
        Err(response) => return response,
    };
    crate::onboarding::project_next_action(&mut status, wallet, account, approval_pending, crate::onboarding::login_retry_after_ms(wallet));
    petal::read_json_value(&status)
});
