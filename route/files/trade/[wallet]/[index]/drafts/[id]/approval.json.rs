petal::route_file!(spec: petal::store_read_spec().caps(&["bloom:store", "bloom:sign.status"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
    let id = match petal::param(ctx, "id") { Ok(value) => value, Err(resp) => return resp };
    match crate::approval::approval_projection(ctx, wallet, &format!("trade/{wallet}/drafts/{id}/approval.json"), &["polymarket.order.poly1271"]) {
        Ok(Some(value)) => petal::read_json_value(&value),
        Ok(None) => petal::error(-3, "no pending signing approval"),
        Err(resp) => resp,
    }
});
