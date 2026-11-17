petal::route_file!(spec: petal::write_spec().caps(&["bloom:store", "bloom:vfs.read"]), read:
    |_ctx: &petal::Ctx| petal::DispatchResponse::Read(br#"write JSON to create a reviewable pUSD funding request, e.g.
{"target_pusd":"10","max_spend":"10"}
from_token: "pUSD" (transfer), "USDC.e" (wrapped 1:1 into pUSD) or "auto" (default: pUSD if the owner holds enough, else USDC.e).
Swap other tokens to USDC.e first.
"#.to_vec()),
    write: |ctx: &petal::Ctx, body: &[u8]| {
    let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
        let wallet = match petal::wallet_param(ctx) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::fund_flow::create_fund_request(wallet, &account, body)
    }
);
