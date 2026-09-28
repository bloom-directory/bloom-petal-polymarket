petal::route_file!(
    spec: petal::write_spec().caps(&["bloom:store"]),
    read: |_ctx: &petal::Ctx| crate::relayer_config::read_relayer_config(),
    write: |_ctx: &petal::Ctx, body: &[u8]| crate::relayer_config::write_relayer_config(body)
);
