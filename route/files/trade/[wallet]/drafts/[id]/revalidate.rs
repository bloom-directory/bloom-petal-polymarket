crate::route_file!(spec: crate::write_spec(), read:
    |ctx: &crate::Ctx| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match crate::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::services::trade::revalidate_hint(wallet, id)
    },
    write: |ctx: &crate::Ctx, body: &[u8]| {
        let wallet = match crate::param(ctx, "wallet") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let id = match crate::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::services::trade::revalidate(wallet, id, body)
    }
);
