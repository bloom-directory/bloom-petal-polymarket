petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:store", "bloom:chain", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    match petal::wallet_param(ctx) { Ok(wallet) => crate::relayer_actions::withdraw_plan(wallet, account), Err(resp) => resp }
});
