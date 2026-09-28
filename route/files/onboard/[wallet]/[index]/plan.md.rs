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
            "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request one owner approval for CLOB auth. Its payload carries Polymarket's server timestamp, so it is rebuilt on every attempt and approved as a single-use approval that is not bound to the bytes: approve within the ceremony window shown in `approval.json` (about five minutes), then retry `begin`; a later retry simply requests a fresh approval. Onboarding then stores CLOB and builder credentials in the private Petal store, deploys the live-factory deposit wallet when needed, and rests at `fund` until pUSD arrives. Once funded, `begin` prepares the deposit-wallet approval batch (unlimited pUSD allowances and outcome-token operator approvals for the Polymarket exchange and collateral-adapter contracts; preview it at `approvals.json`) and requests a second owner approval bound to its exact reviewed bytes; the same ceremony window applies, and the batch itself expires after one hour, after which a fresh batch is prepared and approved. Retry `begin` after approving, and it submits the batch and syncs CLOB buying power before marking the wallet tradeable.\n"
        )
        .into_bytes(),
    )
});
