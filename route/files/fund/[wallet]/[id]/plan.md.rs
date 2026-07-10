petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let id = match petal::param(ctx, "id") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let session = match crate::fund_flow::load_fund_session(wallet, id) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    petal::DispatchResponse::Read(
        format!(
            "# Polymarket funding request {}\n\nWallet: {}\nReceiver: {} ({})\nTarget pUSD: {}\nMax spend: {}\nFrom token: {}\nSlippage bps: {}\nStatus: {}\n",
            session.id,
            session.wallet,
            session.deposit_wallet,
            session.deposit_wallet_source,
            session.target_pusd,
            session.max_spend,
            session.from_token,
            session.slippage_bps,
            session.status
        )
        .into_bytes(),
    )
});
