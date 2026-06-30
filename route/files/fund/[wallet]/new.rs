crate::bloom_write_component!(
    crate::write_spec(),
    |_ctx: &crate::Ctx| crate::DispatchResponse::Read(crate::FUND_NEW_HINT.into()),
    |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::write_fund_new(wallet, body)
    }
);
