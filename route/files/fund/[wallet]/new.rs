crate::route_file!(spec: crate::write_spec(), read:
    |_ctx: &crate::Ctx| crate::services::fund::new_hint(),
    write: |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::services::fund::create(wallet, body)
    }
);
