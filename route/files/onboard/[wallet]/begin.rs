petal::route_file!(spec: petal::write_spec(), read:
    |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(b"write anything here to mint or derive CLOB credentials with the daemon keystore\n".to_vec())
    },
    write: |ctx: &petal::Ctx, _body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::onboarding::begin_onboarding(wallet)
    }
);
