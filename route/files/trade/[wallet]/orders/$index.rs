petal::route_file!(spec: petal::account_read_spec(), ctx_list: |ctx: &petal::Ctx| {
    let wallet = petal::param(ctx, "wallet")?;
    crate::trade_flow_parts::posting::discoverable_order_ids(wallet)
        .map(|ids| ids.into_iter().map(|id| petal::dir(&id)).collect())
});
