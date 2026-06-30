crate::route_file!(spec: crate::static_read_spec(), read: |_ctx: &crate::Ctx| {
    crate::parity_json()
});
