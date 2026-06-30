crate::bloom_read_component!(crate::static_read_spec(), |_ctx: &crate::Ctx| {
    crate::read_meta("parity.json")
});
