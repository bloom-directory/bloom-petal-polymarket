petal::route_file!(spec: petal::signing_write_spec("polymarket.relayer_batch").caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:chain", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write {\"confirm\":true,\"amount\":\"all\"} to prepare or advance the exact pUSD withdrawal batch\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| match petal::param(ctx, "wallet") {
        Ok(wallet) => crate::relayer_actions::confirm_withdraw(ctx, wallet, body), Err(resp) => resp
    }
);
