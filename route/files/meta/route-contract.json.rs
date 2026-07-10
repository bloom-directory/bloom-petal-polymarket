petal::route_file!(spec: petal::static_read_spec(), read: |_ctx: &petal::Ctx| {
    petal::read_json_value(&serde_json::json!({
        "schema": "bloom.polymarket.petal-route-contract.v1",
        "legacy_root_forbidden": "polymarket/",
        "routes": {
            "market": "markets/<slug>/market.json",
            "search": "search/<query>",
            "onboard_begin": "onboard/<wallet>/begin",
            "onboard_review": "onboard/<wallet>/review_intent.json",
            "account_status": "account/<wallet>/status.json",
            "buying_power": "account/<wallet>/buying_power.json",
            "builder_keys": "builder-keys/<wallet>/keys.json",
            "builder_key_revoke": "builder-keys/<wallet>/revoke",
            "enso_settings": "settings/enso-api-key",
            "fund_new": "fund/<wallet>/new",
            "fund_confirm": "fund/<wallet>/<id>/confirm",
            "trade_post": "trade/<wallet>/drafts/<id>/post",
            "arbitrary_order_cancel": "trade/<wallet>/orders/<clob-order-id>/cancel",
            "redeem_confirm": "redeem/<wallet>/<slug>/confirm",
            "revoke_confirm": "revoke-approvals/<wallet>/request/confirm",
            "withdraw_confirm": "withdraw/<wallet>/pusd/confirm",
            "obligations": "obligations/<wallet>.json"
        },
        "generic_ipc_only": [
            "bloom:sign/signing@0.2.0",
            "bloom:tx/outbox@0.1.0",
            "bloom:chain/read@0.1.0"
        ]
    }))
});
