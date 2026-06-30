crate::bloom_write_component!(
    crate::write_spec(),
    |ctx: &crate::Ctx| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match crate::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::read_trade(wallet, "drafts", id, "post")
    },
    |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match crate::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::write_trade_post(wallet, id, body)
    }
);
