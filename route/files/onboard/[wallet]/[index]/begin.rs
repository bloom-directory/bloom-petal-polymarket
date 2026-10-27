petal::route_file!(spec: petal::signing_write_spec("polymarket.clob_auth.v1").caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:vfs.read"]), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write here for CLOB credentials; then write continue for deposit-wallet onboarding. Read status.json for the next action.\n".to_vec())
    },
    write: |ctx: &petal::Ctx, _body: &[u8]| {
        let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::onboarding::begin_onboarding(ctx, wallet, &account)
    }
);
