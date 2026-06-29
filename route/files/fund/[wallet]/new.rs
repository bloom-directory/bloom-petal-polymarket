crate::bloom_write_component!(
|_ctx: &crate::Ctx| crate::DispatchResponse::Read(crate::FUND_NEW_HINT.into()),
    |ctx: &crate::Ctx, body: &[u8]| {
        let Some(wallet) = crate::route_param_or_segment(ctx, "wallet", 1) else {
            return crate::route_invalid("missing wallet");
        };
        crate::write_fund_new(wallet, body)
    }
);
