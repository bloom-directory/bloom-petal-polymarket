petal::route_file!(spec: petal::static_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    petal::DispatchResponse::Read(
        format!(
            "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request one Broker-authorized payload-signing operation for CLOB auth and any required deposit-wallet approval batch, store CLOB and builder credentials in the private Petal store, deploy the live-factory deposit wallet when needed, rest at `fund` until pUSD arrives, then approve and sync CLOB buying power before marking the wallet tradeable.\n"
        )
        .into_bytes(),
    )
});
