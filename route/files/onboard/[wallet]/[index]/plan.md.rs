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
            "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request one owner approval for CLOB authentication. The versioned auth profile permits a fresh server timestamp under its selected-owner, domain, nonce and attestation checks and one-signature limit. Retry `begin` after approval to store the credentials privately. With credentials present, `begin` returns the continuation path.\n\nWrite `continue` to resolve and deploy the deposit wallet and reach the funding stage. After pUSD arrives, `continue` prepares a separate exact approval batch: unlimited pUSD allowances and outcome-token operator approvals for the exchange and collateral adapters. Preview `approvals.json`, approve through `approval.json`, then retry `continue` to submit the unchanged batch and sync CLOB buying power. An expired batch needs a fresh review.\n"
        )
        .into_bytes(),
    )
});
