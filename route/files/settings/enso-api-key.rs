petal::route_file!(spec: petal::write_spec(),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(b"write the Enso API key; the stored secret is never readable\n".to_vec()),
    write: |_ctx: &petal::Ctx, body: &[u8]| crate::account_views::write_enso_api_key(body)
);
