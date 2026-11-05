petal::route_file!(spec: petal::store_read_spec().caps(&["bloom:sign.status"]), read: |ctx: &petal::Ctx| {
    let (wallet, id) = match (petal::wallet_param(ctx), petal::param(ctx, "id")) { (Ok(w), Ok(i)) => (w, i), (Err(r), _) | (_, Err(r)) => return r };
    petal::signing::approval_read(ctx, wallet, &[&format!("trade/{wallet}/drafts/{id}/attempt")])
});
