petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    use crate::polymarket::fees::{FEE_BOUND_NOTE, MarketFee};
    use crate::polymarket::order::format_micro;
    use crate::polymarket::Side;

    let wallet = match petal::wallet_param(ctx) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let id = match petal::param(ctx, "id") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let draft = match crate::trade_flow_parts::storage::load_trade_draft(wallet, id) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    let fee = match &draft.fee {
        None => "not recorded by the build that quoted this draft; write revalidate".to_string(),
        Some(MarketFee::Disabled) => "none (Gamma reports feesEnabled false)".to_string(),
        Some(MarketFee::Enabled { fee_type, rate_ppm, taker_only }) => format!(
            "shares x {} x p x (1 - p), fee type {}, charged to {}",
            format_micro(*rate_ppm),
            fee_type.as_deref().unwrap_or("unnamed"),
            if *taker_only { "takers only" } else { "makers and takers" }
        ),
        Some(MarketFee::Unbounded { fee_type, reason }) => format!(
            "cannot be bounded ({reason}), fee type {}",
            fee_type.as_deref().unwrap_or("unnamed")
        ),
    };
    let fee_bound = match (draft.fee_cap_micro(), draft.side) {
        (Err(err), _) => format!("none, so this draft cannot be posted: {err}"),
        (Ok(cap), Side::Buy) => format!(
            "{} pUSD, so at most {} pUSD leaves the deposit wallet",
            format_micro(cap),
            format_micro(draft.maker_micro.saturating_add(cap))
        ),
        (Ok(_), Side::Sell) => "none from the wallet; a SELL's fee comes out of its pUSD proceeds".to_string(),
    };
    petal::DispatchResponse::Read(
        format!(
            "# Polymarket order draft {}\n\nWallet: {}\nMarket: {}\nQuestion: {}\nOutcome: {}\nToken: {}\nSide: {:?}\nOrder type: {}\nAmount: {}\nPrice bound: {}\nLimit price: {}\nSize: {}\nFee: {}\nFee bound: {}\nStatus: {}\n\n{}\n\nThe draft is live-quoted from Gamma/CLOB and ready for review. Signing and posting are still pending.\n",
            draft.id,
            draft.wallet,
            draft.slug,
            draft.question,
            draft.outcome,
            draft.token_id,
            draft.side,
            draft.order_type.as_str(),
            format_micro(draft.amount_micro),
            format_micro(draft.price_bound_micro),
            format_micro(draft.limit_price_micro),
            format_micro(draft.size_micro),
            fee,
            fee_bound,
            draft.status,
            FEE_BOUND_NOTE
        )
        .into_bytes(),
    )
});
