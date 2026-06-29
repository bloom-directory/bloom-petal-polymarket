crate::bloom_read_component!(
    "trade/[wallet]/drafts/[id]/policy_check.json",
    |ctx: &crate::Ctx| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        let Some(id) = crate::route_param_or_segment(ctx, "id", 3) else {
            return crate::route_invalid("missing id");
        };
        crate::read_trade(wallet, "drafts", id, "policy_check.json")
    }
);
