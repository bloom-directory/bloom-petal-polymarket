crate::bloom_write_component!(
|ctx: &crate::Ctx| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        let Some(id) = crate::route_param_or_segment(ctx, "id", 3) else {
            return crate::route_invalid("missing id");
        };
        crate::read_trade(wallet, "receipts", id, "cancel")
    },
    |ctx: &crate::Ctx, body: &[u8]| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        let Some(id) = crate::route_param_or_segment(ctx, "id", 3) else {
            return crate::route_invalid("missing id");
        };
        crate::write_trade_cancel(wallet, id, body)
    }
);
