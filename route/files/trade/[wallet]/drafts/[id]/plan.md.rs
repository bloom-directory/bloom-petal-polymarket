crate::route_file!(spec: crate::store_read_spec(), read: |ctx: &crate::Ctx| {
    let wallet = match crate::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let id = match crate::param(ctx, "id") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let draft = match crate::load_trade_draft(wallet, id) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    crate::DispatchResponse::Read(
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
            crate::order::format_micro(draft.amount_micro),
            crate::order::format_micro(draft.price_bound_micro),
            crate::order::format_micro(draft.limit_price_micro),
            crate::order::format_micro(draft.size_micro),
            draft.status
        )
        .into_bytes(),
    )
});
