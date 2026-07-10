petal::route_file!(spec: petal::write_spec(),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm or {\"confirm\":true,\"key\":\"<id>\"}\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        match petal::param(ctx, "wallet") {
            Ok(wallet) => crate::account_views::revoke_builder_key(wallet, body),
            Err(resp) => resp,
        }
    }
);
