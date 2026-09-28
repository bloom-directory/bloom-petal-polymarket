petal::route_file!(spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm or {\"cancel\":true} to cancel this discoverable CLOB order\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
        let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
        let order_id = match petal::param(ctx, "clob-order-id") { Ok(value) => value, Err(resp) => return resp };
        crate::trade_flow_parts::posting::cancel_discovered_order(wallet, account, order_id, body)
    }
);
