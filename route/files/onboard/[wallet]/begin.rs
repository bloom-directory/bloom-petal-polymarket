crate::bloom_write_component!(
    crate::write_spec(),
    |ctx: &crate::Ctx| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::read_onboard(wallet, "begin")
    },
    |ctx: &crate::Ctx, _body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::write_onboard_begin(wallet)
    }
);
