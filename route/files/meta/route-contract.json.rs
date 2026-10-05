petal::route_file!(spec: petal::static_read_spec(), read: |_ctx: &petal::Ctx| {
    petal::read_json_value(&serde_json::json!({
        "schema": "bloom.polymarket.petal-route-contract.v1",
        "legacy_root_forbidden": "polymarket/",
        "routes": {
            "market": "markets/<slug>/market.json",
            "search": "search/<query>",
            "onboard_begin": "onboard/<wallet>/<index>/begin",
            "onboard_review": "onboard/<wallet>/<index>/review_intent.json",
            "account_status": "account/<wallet>/<index>/status.json",
            "buying_power": "account/<wallet>/<index>/buying_power.json",
            "builder_keys": "builder-keys/<wallet>/<index>/keys.json",
            "builder_key_revoke": "builder-keys/<wallet>/<index>/revoke",
            "enso_settings": "settings/enso-api-key",
            "relayer_settings": "settings/<wallet>/<index>/relayer.json",
            "builder_code_settings": "settings/builder-code",
            "builder_code_status": "settings/builder-code-status.json",
            "venue_settings": "settings/<wallet>/<index>/venue.toml",
            "fund_new": "fund/<wallet>/<index>/new",
            "fund_confirm": "fund/<wallet>/<index>/<id>/confirm",
            "trade_post": "trade/<wallet>/<index>/drafts/<id>/post",
            "trade_post_builder_code": "trade/<wallet>/<index>/drafts/<id>/post_builder_code",
            "arbitrary_order_cancel": "trade/<wallet>/<index>/orders/<clob-order-id>/cancel",
            "redeem_confirm": "redeem/<wallet>/<index>/<slug>/confirm",
            "revoke_confirm": "revoke-approvals/<wallet>/<index>/request/confirm",
            "withdraw_confirm": "withdraw/<wallet>/<index>/pusd/confirm",
            "obligations": "obligations/<wallet>/<index>/status.json"
        },
        "generic_ipc_only": [
            "bloom:sign/signing@0.2.0",
            "bloom:tx/outbox@0.1.0",
            "bloom:chain/read@0.1.0"
        ]
    }))
});
