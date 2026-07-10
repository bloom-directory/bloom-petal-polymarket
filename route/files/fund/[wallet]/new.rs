petal::route_file!(spec: petal::write_spec(), read:
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"write JSON to create a reviewable pUSD funding request, e.g.
{"target_pusd":"10","max_spend":"100","from_token":"native","slippage_bps":50}
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let wallet = match petal::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::fund_flow::create_fund_request(wallet, body)
    }
);
