petal::route_file!(
    spec: petal::write_spec().caps(&["bloom:store"]),
    read: |_ctx: &petal::Ctx| petal::DispatchResponse::Read(
        b"write {\"maker_bps\":N,\"taker_bps\":M} to set the most a builder code may charge on an order, in basis points of notional. Polymarket keeps a builder's real rates on its profile and serves no API for them, so this ceiling is what the owner approves and what an order is sized against; set it to match the profile. Capped at 50 maker and 100 taker by the venue. A configured builder code without a ceiling cannot post. Write an empty body to clear the override and revert to this release's default, if any\n".to_vec(),
    ),
    write: |_ctx: &petal::Ctx, body: &[u8]| crate::account_views::write_fee_ceiling_override(body)
);
