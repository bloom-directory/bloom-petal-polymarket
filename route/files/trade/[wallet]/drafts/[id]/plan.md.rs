petal::route_file!(spec: petal::store_read_spec(), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
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
    petal::DispatchResponse::Read(
        format!(
            "# Polymarket order draft {}\n\nWallet: {}\nMarket: {}\nQuestion: {}\nOutcome: {}\nToken: {}\nSide: {:?}\nOrder type: {}\nAmount: {}\nPrice bound: {}\nLimit price: {}\nSize: {}\nStatus: {}\n\nThe draft is live-quoted from Gamma/CLOB and ready for review. Signing and posting are still pending.\n",
            draft.id,
            draft.wallet,
            draft.slug,
            draft.question,
            draft.outcome,
            draft.token_id,
            draft.side,
            draft.order_type.as_str(),
            crate::polymarket::order::format_micro(draft.amount_micro),
            crate::polymarket::order::format_micro(draft.price_bound_micro),
            crate::polymarket::order::format_micro(draft.limit_price_micro),
            crate::polymarket::order::format_micro(draft.size_micro),
            draft.status
        )
        .into_bytes(),
    )
});
