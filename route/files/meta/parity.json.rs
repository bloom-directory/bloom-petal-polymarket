crate::bloom_read_component!("meta/parity.json", |_ctx: &crate::Ctx| {
    crate::read_meta("parity.json")
});
