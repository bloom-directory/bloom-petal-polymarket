petal::route_file!(spec: petal::write_spec(),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm to prepare or advance the exact approval-revocation batch\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| match petal::param(ctx, "wallet") {
        Ok(wallet) => crate::relayer_actions::confirm_revoke(wallet, body), Err(resp) => resp
    }
);
