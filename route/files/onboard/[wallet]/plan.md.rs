crate::route_file!(spec: crate::static_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::validate_wallet_name(wallet) {
        return crate::error(-3, e.to_string());
    }
    crate::DispatchResponse::Read(
        format!(
            "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request daemon-keystore signatures for CLOB auth and any required deposit-wallet approval batch, store CLOB and builder credentials in the private petal store, deploy the live-factory deposit wallet when needed, rest at `fund` until pUSD arrives, then approve and sync CLOB buying power before marking the wallet tradeable.\n"
        )
        .into_bytes(),
    )
});
