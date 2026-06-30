crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| {
        crate::DispatchResponse::Read(b"write anything here to mint or derive CLOB credentials with the daemon keystore\n".to_vec())
    },
    write: |ctx: &crate::Ctx, _body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::begin_onboarding(wallet)
    }
);
