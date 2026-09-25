petal::route_file!(spec: petal::static_read_spec().caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    match crate::account::wallet_param(ctx) { Ok(wallet) => crate::relayer_actions::revoke_plan(wallet), Err(resp) => resp }
});
