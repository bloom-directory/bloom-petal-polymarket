crate::bloom_write_component!(
|ctx: &crate::Ctx| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        crate::read_onboard(wallet, "begin")
    },
    |ctx: &crate::Ctx, _body: &[u8]| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        crate::write_onboard_begin(wallet)
    }
);
