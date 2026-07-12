petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:store", "bloom:vfs.read", "bloom:vfs.write"]), read: |ctx: &petal::Ctx| {
    match petal::param(ctx, "wallet") { Ok(wallet) => crate::relayer_actions::withdraw_plan(wallet), Err(resp) => resp }
});
