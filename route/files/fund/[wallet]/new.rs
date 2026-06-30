crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| crate::DispatchResponse::Read(br#"write JSON to create a reviewable pUSD funding request, e.g.
{"target_pusd":"10","max_spend":"100","from_token":"native","slippage_bps":50}
"#.to_vec()),
    write: |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::create_fund_request(wallet, body)
    }
);
