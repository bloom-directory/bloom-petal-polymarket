petal::route_file!(spec: petal::signing_write_spec("polymarket.clob_auth.v1").caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:vfs.read"]), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write here for CLOB credentials; then write continue for deposit-wallet onboarding. Read status.json for the next action. Write {\"refresh\":true} to refresh existing credentials.\n".to_vec())
    },
    write: |ctx: &petal::Ctx, body: &[u8]| {
        if body.len() > 1024 { return petal::error(-3, "begin request is too large"); }
        #[derive(serde::Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        struct BeginRequest { #[serde(default)] refresh: bool }
        let request = if body.is_empty() { BeginRequest::default() } else {
            match serde_json::from_slice::<BeginRequest>(body) { Ok(value) => value, Err(error) => return petal::error(-3, format!("begin JSON: {error}")) }
        };
        let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::onboarding::begin_onboarding(ctx, wallet, &account, request.refresh)
    }
);
