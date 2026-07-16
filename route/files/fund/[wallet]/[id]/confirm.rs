petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:tx.outbox", "bloom:chain", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm to prepare or advance the persisted outbox funding plan\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
        let id = match petal::param(ctx, "id") { Ok(value) => value, Err(resp) => return resp };
        crate::fund_flow::confirm_fund_request(wallet, id, body)
    }
);
