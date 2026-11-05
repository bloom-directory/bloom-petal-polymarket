petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:http", "bloom:store", "bloom:chain", "bloom:vfs.read", "bloom:sign.status"]), read: |ctx: &petal::Ctx| {
    let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
    let wallet = match petal::wallet_param(ctx) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    let mut status = match crate::infra_parts::host_calls::wallet_address(&account) {
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
    status["approvals"]["preview_path"] = serde_json::json!(format!("onboard/{wallet}/{}/approvals.json", account.number()));
    let login = match crate::onboarding::login_scope(&mut petal::signing::CtxHost(ctx), wallet) { Ok(s) => s, Err(e) => return e.into() };
    let batch = match crate::polymarket::relayer_batch::current_scope(&mut petal::signing::CtxHost(ctx), &format!("onboard/{wallet}")) { Ok(s) => s, Err(r) => return r };
    let approval_pending = match petal::signing::approval_read(ctx, wallet, &[&login, batch.as_deref().unwrap_or("onboard/no-batch")]) {
        petal::DispatchResponse::Read(_) => true,
        petal::DispatchResponse::Error { code: -3, .. } => false,
        other => return other,
    };
    crate::onboarding::project_next_action(&mut status, wallet, account.number(), approval_pending);
    petal::read_json_value(&status)
});
