petal::route_file!(
    spec: petal::signing_write_spec("polymarket.onboard").caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:chain", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write here after begin to deploy the selected deposit wallet; fund it with fund/<wallet>/<index>/new, then write here again to approve\n".to_vec())
    },
    write: |ctx: &petal::Ctx, _body: &[u8]| {
        let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
        let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
        crate::onboarding::continue_onboarding(ctx, wallet, &account)
    }
);
