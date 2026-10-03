petal::route_file!(spec: petal::store_read_spec().caps(&["bloom:store"]), read: |_ctx: &petal::Ctx| {
    crate::account_views::read_builder_code_status()
});
