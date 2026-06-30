crate::route_file!(spec: crate::static_read_spec(), read: |_ctx: &crate::Ctx| {
    crate::read_json_value(&serde_json::json!({
        "kind": "polymarket_v2_petal_parity",
        "mount": "apps/polymarket",
        "status": "v2_implementation",
        "graduation_ready": true,
        "no_on_chain_code_touched_by_local_petal": true,
        "secret_storage": {
            "clob_credentials": "private_store_only",
            "builder_credentials": "private_store_only",
            "public_vfs_receipts": "redacted_summaries_only"
        },
        "implemented": [
            {
                "id": "market_reads",
                "surface": ["markets/*/market.json", "markets/*/book.json", "markets/*/prices.json"],
                "evidence": "HTTP via manifest allowlisted Gamma/CLOB reads"
            },
            {
                "id": "positions_and_account_reads",
                "surface": ["positions/*/*.json", "account/*/portfolio.json", "account/*/orders.json"],
                "evidence": "wallet-resolved Data API and L2 CLOB account reads"
            },
            {
                "id": "onboarding_credentials",
                "surface": ["onboard/*/begin", "onboard/*/status.json", "onboard/*/approvals.json"],
                "evidence": "geoblock-gated live factory deposit-wallet resolution plus CLOB auth signature through sign_hash and private credential storage"
            },
            {
                "id": "factory_resolved_deposit_wallet",
                "surface": ["onboard/*/status.json", "onboard/*/approvals.json", "fund/*/new", "trade/*/drafts/*/revalidate", "trade/*/drafts/*/post"],
                "evidence": "funding and posting paths require a persisted live_factory_resolved deposit wallet instead of the display-only local CREATE2 estimate"
            },
            {
                "id": "read_only_onboarding_stage_probes",
                "surface": ["onboard/*/status.json", "account/*/portfolio.json", "trade/*/drafts/*/revalidate", "trade/*/drafts/*/post"],
                "evidence": "local status recomputes deployed/funded/approved/credentialed/CLOB-synced readiness from mediated chain reads plus private credentials; posting requires stage=complete"
            },
            {
                "id": "onboarding_relayer_deploy_approve_sync",
                "surface": ["onboard/*/begin", "onboard/*/status.json"],
                "evidence": "local begin auto-mints private builder credentials, submits relayer WALLET-CREATE and signed V2 approval WALLET batches when live probes show they are needed, polls confirmation, rests at fund when pUSD is absent, and calls CLOB balance-allowance update before marking complete"
            },
            {
                "id": "buy_posting",
                "surface": ["trade/*/drafts/*/revalidate", "trade/*/drafts/*/post"],
                "evidence": "final-review-bound POLY_1271 buy posting with private receipt/audit records"
            },
            {
                "id": "authoritative_sell_posting",
                "surface": ["trade/*/drafts/*/revalidate", "trade/*/drafts/*/post"],
                "evidence": "sell posting is gated by CLOB conditional balance and chain CTF balanceOf/isApprovedForAll reads through the host-mediated chain/account surfaces; Data API holdings are recorded as corroborating evidence only"
            },
            {
                "id": "ambiguous_post_reconciliation",
                "surface": ["trade/*/drafts/*/post"],
                "evidence": "lost POST outcomes reconcile only against strongly matched L2 /data/orders responses"
            },
            {
                "id": "resting_gtc_cancel",
                "surface": ["trade/*/receipts/*/cancel"],
                "evidence": "GTC buy posting is paired with exact DELETE /order cancel from private receipt order id"
            },
            {
                "id": "local_policy_and_daily_cap",
                "surface": ["trade/*/drafts/*/policy_check.json"],
                "evidence": "wallet policy, receipt-audit parity, and daily exposure checks fail closed"
            }
        ],
        "remaining_blockers": [],
        "graduation_evidence": [
            "compiled wasm router smoke covers apps/polymarket market, search, position, account, onboarding, funding, buy, sell, reconcile, cancel, and public redaction surfaces",
            "public VFS reads are swept for private CLOB credentials, builder credentials, API keys/passphrases, raw echoed signatures, raw CLOB response bodies, and echoed signature payloads",
            "adversarial review findings are fixed or documented in docs/reviews/2026-06-23-local-petal-plugins-closeout.md",
            "GTD order posting remains deferred because the existing Polymarket behavior also rejects GTD orders"
        ],
        "native_unsupported_or_deferred": [
            {
                "id": "gtd_orders",
                "status": "not_required_for_current_parity",
                "reason": "the current Polymarket surface rejects GTD orders; the v2 petal also rejects GTD pending a future expiry policy"
            }
        ],
        "graduation_requirements": [
            "all implemented surfaces pass focused and broader validation",
            "adversarial review has no unresolved findings",
            "public VFS reads contain no CLOB credential secret or raw signed order body",
            "remaining blockers are either implemented or explicitly accepted before removing the legacy native polymarket surface"
        ]
    }))
});
