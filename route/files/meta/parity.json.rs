crate::route_file!(spec: crate::static_read_spec(), read: |_ctx: &crate::Ctx| {
    crate::read_meta("parity.json")
});
