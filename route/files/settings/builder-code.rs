petal::route_file!(
    spec: petal::write_spec().caps(&["bloom:store"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(
        b"write a 0x hex builder attribution code (up to 32 bytes) from your Polymarket Builder Profile to override the default used on every order when this release has no embedded default, or to replace it; write an empty body to clear the override and revert to the embedded release default, if any\n".to_vec(),
    ),
    write: |_ctx: &petal::Ctx, body: &[u8]| crate::account_views::write_builder_code_override(body)
);
