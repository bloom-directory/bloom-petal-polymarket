petal::route_file!(spec: petal::static_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    petal::DispatchResponse::Read(
        format!(
            "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request owner approval, one signature at a time: first the deposit-wallet approval batch (bound to its exact reviewed bytes), then CLOB auth (a single-use approval, so its timestamp is rebuilt after you approve). Retry `begin` after each approval. Onboarding then stores CLOB and builder credentials in the private Petal store, deploys the live-factory deposit wallet when needed, rests at `fund` until pUSD arrives, then sets approvals and syncs CLOB buying power before marking the wallet tradeable.\n"
        )
        .into_bytes(),
    )
});
