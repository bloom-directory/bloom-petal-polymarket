petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm or {\"confirm\":true,\"key\":\"<id>\"}\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
        match petal::wallet_param(ctx) {
            Ok(wallet) => crate::account_views::revoke_builder_key(wallet, account, body),
            Err(resp) => resp,
        }
    }
);
