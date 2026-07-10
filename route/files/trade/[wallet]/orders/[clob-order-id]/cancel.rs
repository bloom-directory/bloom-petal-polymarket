petal::route_file!(spec: petal::write_spec(),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write confirm or {\"cancel\":true} to cancel this discoverable CLOB order\n".to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") { Ok(value) => value, Err(resp) => return resp };
        let order_id = match petal::param(ctx, "clob-order-id") { Ok(value) => value, Err(resp) => return resp };
        crate::trade_flow_parts::posting::cancel_discovered_order(wallet, order_id, body)
    }
);
