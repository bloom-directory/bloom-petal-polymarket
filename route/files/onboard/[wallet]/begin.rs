crate::route_file!(spec: crate::write_spec(), read:
    |ctx: &crate::Ctx| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::services::onboard::begin_read(wallet)
    },
    write: |ctx: &crate::Ctx, _body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::services::onboard::begin_write(wallet)
    }
);
