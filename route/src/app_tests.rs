#[cfg(test)]
mod tests {
    use crate::app_types::{StoreTradeDraft, TradeSnapshot};
    use crate::infra_parts::util::{url_with_query, validate_relative_path};
    use crate::polymarket::order::OrderType;
    use crate::polymarket::{Market, Side};
    use crate::prelude::*;
    use crate::trade_flow_parts::pricing::{build_trade_quote, choose_trade_limit};
    use alloy::primitives::Address;
    use petal::DispatchResponse;

    fn market() -> Market {
        Market {
            id: "1".into(),
            slug: "example".into(),
            question: "Example?".into(),
            condition_id: "0xabc".into(),
            clob_token_ids: vec!["111".into(), "222".into()],
            outcomes: vec!["Yes".into(), "No".into()],
            outcome_prices: Vec::new(),
            active: true,
            closed: false,
            enable_order_book: true,
            order_price_min_tick_size: None,
            order_min_size: None,
            neg_risk: false,
        }
    }

    fn snapshot(best_ask_micro: Option<u64>, best_bid_micro: Option<u64>) -> TradeSnapshot {
        TradeSnapshot {
            market: market(),
            outcome: "YES".into(),
            token_id: "111".into(),
            neg_risk: false,
            tick_micro: 10_000,
            min_size_micro: 5_000_000,
            best_ask_micro,
            best_bid_micro,
            active: true,
            closed: false,
            order_book_enabled: true,
        }
    }

    fn policy_ctx() -> LocalPolymarketOrderCtx {
        LocalPolymarketOrderCtx {
            slug: "example".into(),
            condition_id: "0xabc".into(),
            side: LocalPolicySide::Buy,
            amount_microusd: 10_000_000,
            fee_microusd: 0,
            limit_price_micro: 695_000,
            active: true,
            closed: false,
            order_book_enabled: true,
            binary_outcomes: true,
            neg_risk: false,
            receipt_store_readable: true,
            daily_posted_microusd: Some(0),
        }
    }

    fn draft() -> StoreTradeDraft {
        StoreTradeDraft {
            id: "0001".into(),
            wallet: "alice".into(),
            slug: "example".into(),
            question: "Example?".into(),
            condition_id: "0xabc".into(),
            outcome: "YES".into(),
            token_id: "111".into(),
            side: Side::Buy,
            order_type: OrderType::FAK,
            amount_micro: 1_000_000,
            price_bound_micro: 100_000,
            limit_price: None,
            marketable: true,
            limit_price_micro: 90_000,
            size_micro: 11_111_100,
            maker_micro: 1_000_000,
            taker_micro: 11_111_100,
            tick_micro: 10_000,
            min_order_size_micro: 1_000_000,
            neg_risk: false,
            active: true,
            closed: false,
            order_book_enabled: true,
            binary_outcomes: true,
            best_ask_micro: Some(90_000),
            best_bid_micro: Some(80_000),
            book_snapshot_secs: 1,
            builder_fee_rate_bps: None,
            builder_fee_micro: None,
            builder_code: None,
            status: "signed".into(),
            salt: Some(42),
            clob_order_id: None,
            clob_status: None,
            last_error: None,
        }
    }

    fn clob_manifest_allows(method: &str, path: &str) -> bool {
        let manifest: toml::Value = toml::from_str(include_str!("../../petal.toml")).unwrap();
        manifest
            .get("net")
            .and_then(|net| net.get("allow"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .any(|entry| {
                entry.get("host").and_then(toml::Value::as_str) == Some("clob.polymarket.com")
                    && entry
                        .get("methods")
                        .and_then(toml::Value::as_array)
                        .is_some_and(|methods| {
                            methods
                                .iter()
                                .any(|allowed| allowed.as_str() == Some(method))
                        })
                    && entry
                        .get("paths")
                        .and_then(toml::Value::as_array)
                        .is_some_and(|paths| {
                            paths.iter().any(|allowed| allowed.as_str() == Some(path))
                        })
            })
    }

    /// The fee-bearing class a builder-coded order signs under. Broker needs
    /// an operation class to be uniformly fee-bearing or uniformly fee-free,
    /// so this cannot share `polymarket.order.poly1271` with plain orders.
    /// Bloom catalogues this exact string with the polygon/pusd fee asset in
    /// `FEE_BEARING_OPERATION_CLASSES`; if either side renames it, enrollment
    /// leaves the class fee-free and signing answers `FEE_NOT_ALLOWED`.
    #[test]
    fn builder_code_order_class_is_declared_separately_from_the_plain_order_class() {
        let manifest: toml::Value = toml::from_str(include_str!("../../petal.toml")).unwrap();
        let intents = manifest
            .get("sign")
            .and_then(|sign| sign.get("allowed_intents"))
            .and_then(toml::Value::as_array)
            .expect("petal.toml declares [sign].allowed_intents")
            .iter()
            .filter_map(toml::Value::as_str)
            .collect::<Vec<_>>();
        assert!(
            intents.contains(&"polymarket.order.poly1271"),
            "plain orders keep their fee-free class"
        );
        assert!(
            intents.contains(&"polymarket.builder_code_order.poly1271"),
            "builder-code orders need their own fee-bearing class"
        );
        assert_ne!(
            "polymarket.order.poly1271",
            "polymarket.builder_code_order.poly1271"
        );
    }

    /// A spend cap should bound what leaves the wallet. Testing only the
    /// helper left every call site free to go back to the bare notional, so
    /// this drives the evaluator and asserts the cap actually denies.
    #[test]
    fn a_spend_cap_denies_when_the_fee_pushes_the_order_over_it() {
        use crate::trade_flow_parts::policy::evaluate_local_polymarket_order;
        let policy = crate::app_types::PolymarketVenueConfig {
            enabled: true,
            max_order_usd: Some(10_000_000),
            ..Default::default()
        };

        // Notional alone is exactly at the cap: allowed.
        let ctx = policy_ctx();
        assert_eq!(ctx.total_spend_microusd(), 10_000_000);
        let denied = |checks: Vec<crate::app_types::LocalPolicyCheck>| {
            checks.iter().any(|check| {
                check.rule == "polymarket.max_order_usd"
                    && matches!(check.outcome, crate::app_types::LocalPolicyOutcome::Deny)
            })
        };
        assert!(
            !denied(evaluate_local_polymarket_order(&policy, &ctx)),
            "notional at the cap must be allowed"
        );

        // The same notional with a fee is over it, and must be denied.
        let mut with_fee = policy_ctx();
        with_fee.fee_microusd = 1;
        assert_eq!(with_fee.total_spend_microusd(), 10_000_001);
        assert!(
            denied(evaluate_local_polymarket_order(&policy, &with_fee)),
            "the cap must measure the fee, not just the notional"
        );
    }

    /// For a sell or a limit buy a ceiling change does not move `maker` or
    /// `taker`, so the review intent has to compare the fee itself or a change
    /// between the review and the signature would pass unnoticed.
    #[test]
    fn the_review_intent_notices_a_fee_only_change() {
        use crate::trade_flow_parts::revalidate::review_intent_matches_draft;
        let mut draft = draft();
        draft.side = Side::Buy;
        draft.builder_fee_rate_bps = Some(100);
        draft.builder_fee_micro = Some(100_000);
        draft.builder_code = Some("0x000000000000000000000000000000000000000a".to_owned());
        let owner: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let funder: Address = "0x0000000000000000000000000000000000000002"
            .parse()
            .unwrap();
        let policy_check = serde_json::json!({"policy_status": "ok"});
        let review = crate::trade_flow_parts::revalidate::with_review_fee_fields(
            serde_json::json!({
            "wallet": draft.wallet,
            "draft_id": draft.id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "slug": draft.slug,
            "condition_id": draft.condition_id,
            "outcome": draft.outcome,
            "token_id": draft.token_id,
            "side": "BUY",
            "order_type": draft.order_type.as_str(),
            "limit_price": crate::polymarket::order::format_micro(draft.limit_price_micro),
            "size": crate::polymarket::order::format_micro(draft.size_micro),
            "maker": crate::polymarket::order::format_micro(draft.maker_micro),
            "taker": crate::polymarket::order::format_micro(draft.taker_micro),
                "policy_status": "ok",
                "neg_risk": draft.neg_risk,
            }),
            // Merged by the same helper the writer uses, so this fixture
            // cannot drift from production the way the first one did, and a
            // fee field added later lands here without being hand-typed.
            &draft,
        );

        // The draft the review was taken from still matches it.
        review_intent_matches_draft(&review, &draft, owner, funder, &policy_check, None)
            .expect("the draft its review was built from must match");

        // Only the fee moves, and it is caught. Nothing else differs, so a
        // comparison that ignored the fee would pass this.
        for mutate in [
            |d: &mut StoreTradeDraft| d.builder_fee_micro = Some(50_000),
            |d: &mut StoreTradeDraft| d.builder_fee_rate_bps = Some(50),
            // The beneficiary moves without the amount moving at all, which
            // the fee comparison alone cannot see.
            |d: &mut StoreTradeDraft| {
                d.builder_code = Some("0x00000000000000000000000000000000000000ff".to_owned());
            },
        ] {
            let mut changed = draft.clone();
            mutate(&mut changed);
            assert!(
                review_intent_matches_draft(&review, &changed, owner, funder, &policy_check, None)
                    .is_err(),
                "a fee-only change must not pass the review comparison"
            );
        }
    }

    /// With no builder code the three fee fields are absent, and the writer
    /// used to emit JSON `null` while the comparison read a string, so no post
    /// could match at all — including a plain order, which worked before fees
    /// existed. The fields are strings on both sides now; this holds it.
    #[test]
    fn a_draft_with_no_builder_code_still_matches_its_own_review() {
        use crate::trade_flow_parts::revalidate::{
            review_fee_fields, review_intent_matches_draft, with_review_fee_fields,
        };
        let mut draft = draft();
        draft.side = Side::Buy;
        assert_eq!(draft.builder_fee_micro, None);
        assert!(
            review_fee_fields(&draft)
                .iter()
                .all(|(_, rendered)| rendered == "none"),
            "every fee field must render as the string `none`, not JSON null"
        );

        let owner: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let funder: Address = "0x0000000000000000000000000000000000000002"
            .parse()
            .unwrap();
        let review = with_review_fee_fields(
            serde_json::json!({
                "wallet": draft.wallet,
                "draft_id": draft.id,
                "owner": owner.to_checksum(None),
                "funder": funder.to_checksum(None),
                "slug": draft.slug,
                "condition_id": draft.condition_id,
                "outcome": draft.outcome,
                "token_id": draft.token_id,
                "side": "BUY",
                "order_type": draft.order_type.as_str(),
                "limit_price": crate::polymarket::order::format_micro(draft.limit_price_micro),
                "size": crate::polymarket::order::format_micro(draft.size_micro),
                "maker": crate::polymarket::order::format_micro(draft.maker_micro),
                "taker": crate::polymarket::order::format_micro(draft.taker_micro),
                "policy_status": "ok",
                "neg_risk": draft.neg_risk,
            }),
            &draft,
        );
        review_intent_matches_draft(
            &review,
            &draft,
            owner,
            funder,
            &serde_json::json!({"policy_status": "ok"}),
            None,
        )
        .expect("a plain order must be postable");
    }

    /// The daily cap's fee term was dead: `StoreTradeReceipt` had no fee
    /// field, so the sum always added zero. A receipt must carry it, and the
    /// policy view must read the same field off the serialized form.
    #[test]
    fn a_receipt_records_its_fee_and_the_policy_view_reads_it() {
        let receipt = serde_json::json!({
            "draft_id": "0001",
            "wallet": "alice",
            "slug": "example",
            "token_id": "111",
            "side": "BUY",
            "order_type": "FAK",
            "funder": null,
            "signature_type": 3,
            "amount_microusd": 10_000_000,
            "fee_microusd": 175_000,
            "limit_price_micro": 500_000,
            "size_micro": 20_000_000,
            "clob_status": "matched",
            "posted_ms": 1_u64,
        });
        let view: crate::app_types::StoreTradeReceiptPolicy =
            serde_json::from_value(receipt.clone()).expect("the policy view reads a receipt");
        assert_eq!(view.amount_microusd, 10_000_000);
        assert_eq!(view.fee_microusd, 175_000, "the fee term must not be dead");

        // A receipt written before fees were accounted still parses as zero.
        let mut old = receipt;
        old.as_object_mut().unwrap().remove("fee_microusd");
        let view: crate::app_types::StoreTradeReceiptPolicy =
            serde_json::from_value(old).expect("an older receipt must still parse");
        assert_eq!(view.fee_microusd, 0);
    }

    /// `quoted_fee_fields` feeds the plan, the review intent and the policy
    /// cap. Returning nothing unconditionally left all three blank with the
    /// whole suite green.
    #[test]
    fn quoted_fee_fields_reports_the_fee_it_was_configured_with() {
        use crate::trade_flow_parts::pricing::{
            SpendFeeAttribution, SpendFeeInputs, quoted_fee_fields,
        };
        const CODE: &str = "0x00000000000000000000000000000000000000000000000000000000000000ab";
        let configured = SpendFeeAttribution {
            inputs: SpendFeeInputs { builder_bps: 100 },
            code_hex: CODE.to_owned(),
        };
        let quote = crate::polymarket::order::LimitQuote {
            side: Side::Buy,
            price_micro: 500_000,
            size_micro: 20_000_000,
            maker_micro: 10_000_000,
            taker_micro: 20_000_000,
        };
        assert_eq!(
            quoted_fee_fields(Some(&configured), &quote),
            (Some(100), Some(100_000), Some(CODE.to_owned()))
        );
        assert_eq!(quoted_fee_fields(None, &quote), (None, None, None));

        // A sell is where par and the USD leg diverge, so this is the case
        // that pins the par binding. A buy cannot: its maker leg IS its USD
        // leg, so substituting one for the other is invisible there.
        let sell = crate::polymarket::order::LimitQuote {
            side: Side::Sell,
            price_micro: 100_000,
            size_micro: 10_000_000,
            maker_micro: 10_000_000,
            taker_micro: 1_000_000,
        };
        assert_eq!(
            quoted_fee_fields(Some(&configured), &sell),
            (Some(100), Some(100_000), Some(CODE.to_owned())),
            "a sell's fee must be priced on par, not on its USD leg"
        );
    }

    /// A market buy's sizing already folded the fee into the requested amount,
    /// so counting it again denied roughly 1% early on every market buy.
    #[test]
    fn a_market_buy_does_not_count_its_fee_twice_against_the_cap() {
        use crate::trade_flow_parts::pricing::fee_on_top_of_amount_micro;
        let mut draft = draft();
        draft.builder_fee_micro = Some(99_000);

        draft.side = Side::Buy;
        for order_type in [OrderType::FAK, OrderType::FOK] {
            draft.order_type = order_type;
            assert_eq!(
                fee_on_top_of_amount_micro(&draft),
                0,
                "{order_type:?} already fits its fee inside the requested amount"
            );
        }
        // A resting buy pays the fee on top, so it must still be counted.
        draft.order_type = OrderType::GTC;
        assert_eq!(fee_on_top_of_amount_micro(&draft), 99_000);
    }

    #[test]
    fn path_validation_rejects_escape_segments() {
        assert!(validate_relative_path("").is_ok());
        assert!(validate_relative_path("markets/example/market.json").is_ok());
        assert!(validate_relative_path("../wallets").is_err());
        assert!(validate_relative_path("markets//book.json").is_err());
        assert!(validate_relative_path("markets\\evil").is_err());
    }

    #[test]
    fn route_sources_are_individual_and_index_only_for_directories() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("files");
        let mut stack = vec![root];
        let mut routes = Vec::new();

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read route dir") {
                let path = entry.expect("read route entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                    continue;
                }
                assert_ne!(
                    path.file_name().and_then(std::ffi::OsStr::to_str),
                    Some("$list.rs"),
                    "$list.rs route sources are no longer supported"
                );
                routes.push(path);
            }
        }

        assert_eq!(routes.len(), 113);
        assert!(routes.iter().any(|path| path.ends_with("$index.rs")));
        assert!(
            routes
                .iter()
                .any(|path| path
                    .ends_with("trade/[wallet]/[index]/drafts/[id]/post_builder_code.rs")),
            "builder-coded posting keeps its own route so its class can be fee-bearing"
        );
        assert!(
            routes
                .iter()
                .any(|path| path.ends_with("trade/[wallet]/[index]/receipts/[id]/cancel.rs"))
        );
        assert!(
            routes
                .iter()
                .any(|path| path.ends_with("meta/route-contract.json.rs"))
        );
        assert!(
            routes.iter().any(
                |path| path.ends_with("onboard/[wallet]/[index]/review_relayer_intent.json.rs")
            )
        );
        assert!(
            !routes
                .iter()
                .any(|path| path.ends_with("meta/parity.json.rs"))
        );
    }

    #[test]
    fn manifest_allows_only_required_clob_methods_for_runtime_paths() {
        assert!(clob_manifest_allows("post", "/auth/builder-api-key"));
        assert!(clob_manifest_allows("get", "/balance-allowance/update"));
        assert!(clob_manifest_allows("post", "/balance-allowance/update"));
        assert!(clob_manifest_allows("get", "/balance-allowance"));
        assert!(clob_manifest_allows("delete", "/order"));

        assert!(clob_manifest_allows("get", "/auth/builder-api-key"));
        assert!(clob_manifest_allows("delete", "/auth/builder-api-key"));
        assert!(clob_manifest_allows("get", "/time"));
        assert!(!clob_manifest_allows("post", "/data/orders"));
        assert!(!clob_manifest_allows("delete", "/balance-allowance/update"));
    }

    #[test]
    fn runtime_contract_uses_structured_signing_and_outbox_without_parity_marker() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let wit = bloom_petal_contract::WIT_FILES
            .iter()
            .find_map(|(path, contents)| (*path == "route.wit").then_some(*contents))
            .expect("canonical route WIT");
        let wit = std::str::from_utf8(wit).expect("route WIT is UTF-8");
        assert!(wit.contains("package bloom:route@0.1.0"));
        assert!(wit.contains("bloom:sign/signing@0.2.0"));
        assert!(!wit.contains("bloom:sign/signing@0.1.0"));
        assert!(wit.contains("bloom:tx/outbox@0.1.0"));
        assert!(!root.join("route/files/meta/parity.json.rs").exists());
    }

    #[test]
    fn venue_configuration_is_petal_owned_not_wallet_policy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read_to_string(root.join("src/trade_flow_parts/policy.rs"))
            .expect("venue configuration source");
        assert!(source.contains("settings/{wallet}/venue.toml"));
        assert!(source.contains("petal::sdk::store_get"));
        assert!(!source.contains("wallets/{wallet}/policy.toml"));
        assert!(!source.contains("petal::sdk::vfs_read"));
        assert!(
            root.join("files/settings/[wallet]/[index]/venue.toml.rs")
                .is_file()
        );
    }

    #[test]
    fn wasm_relayer_paths_do_not_use_blocking_sleep() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infra_parts/relayer.rs"),
        )
        .expect("relayer source");
        assert!(!source.contains("thread::sleep"));
    }

    #[test]
    fn completed_onboard_refresh_clears_stale_in_flight_marker() {
        let previous = serde_json::json!({
            "deploy_tx_id": "tx-d",
            "approve_tx_id": "tx-a",
            "relayer_auth": "builder_key_auto",
            "in_flight_deadline_ms": "123",
            "last_error": "old error",
            "status_updated_ms": "456"
        });
        let mut refreshed = serde_json::json!({
            "stage": "complete",
            "tradeable": true
        });

        preserve_onboard_metadata(&previous, &mut refreshed);

        assert_eq!(refreshed["deploy_tx_id"], "tx-d");
        assert_eq!(refreshed["approve_tx_id"], "tx-a");
        assert_eq!(refreshed["relayer_auth"], "builder_key_auto");
        assert_eq!(refreshed["status_updated_ms"], "456");
        assert!(refreshed.get("in_flight_deadline_ms").is_none());
        assert!(refreshed.get("last_error").is_none());
    }

    #[test]
    fn unmatched_resting_receipts_still_count_as_exposure() {
        assert!(clob_status_excluded_from_daily_cap(
            "unmatched",
            Some(OrderType::FAK)
        ));
        assert!(!clob_status_excluded_from_daily_cap(
            "unmatched",
            Some(OrderType::GTC)
        ));
        assert!(!clob_status_excluded_from_daily_cap("unmatched", None));
        assert!(clob_status_excluded_from_daily_cap(
            "rejected",
            Some(OrderType::GTC)
        ));
    }

    #[test]
    fn open_order_reconciliation_requires_salt_and_stable_fields() {
        let draft = draft();
        let funder: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
            .parse()
            .unwrap();
        let matched = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY",
            "orderType": "FAK",
            "makerAmount": "1000000",
            "takerAmount": "11111100"
        });
        assert!(open_order_matches_draft(&matched, &draft, funder, 42));

        let wrong_token = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "222",
            "side": "BUY"
        });
        assert!(!open_order_matches_draft(&wrong_token, &draft, funder, 42));

        let weak = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "salt": 42,
            "asset_id": "111"
        });
        assert!(!open_order_matches_draft(&weak, &draft, funder, 42));

        let cancelled = serde_json::json!({
            "id": "order-1",
            "status": "cancelled",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY"
        });
        assert!(!open_order_matches_draft(&cancelled, &draft, funder, 42));

        let contradictory_nested = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY",
            "order": {
                "tokenId": "222",
                "side": "SELL"
            }
        });
        assert!(!open_order_matches_draft(
            &contradictory_nested,
            &draft,
            funder,
            42
        ));

        let malformed_nested = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY",
            "order": {
                "salt": "wrong"
            }
        });
        assert!(!open_order_matches_draft(
            &malformed_nested,
            &draft,
            funder,
            42
        ));

        let empty_id = serde_json::json!({
            "id": "   ",
            "status": "live",
            "salt": 42,
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY"
        });
        assert!(!open_order_matches_draft(&empty_id, &draft, funder, 42));

        let nested = serde_json::json!({
            "id": "order-1",
            "status": "live",
            "order": {
                "salt": "42",
                "maker": funder.to_checksum(None),
                "tokenId": "111"
            }
        });
        assert_eq!(
            find_matching_open_order(&serde_json::json!({"data": [nested]}), &draft, funder, 42)
                .and_then(|order| clob_response_order_id(&order)),
            Some("order-1".into())
        );
    }

    #[test]
    fn url_query_encoding_is_canonical() {
        let url = url_with_query(
            "https://gamma-api.polymarket.com/public-search",
            &[("q", "hello world")],
        );
        assert_eq!(
            url,
            "https://gamma-api.polymarket.com/public-search?q=hello+world"
        );
    }

    #[test]
    fn trade_quote_uses_live_best_ask_and_market_buy_rounding() {
        let snap = snapshot(Some(695_000), Some(690_000));
        let limit = choose_trade_limit(Side::Buy, true, 700_000, 700_000, &snap).unwrap();
        assert_eq!(limit, 690_000);

        let quote =
            build_trade_quote(Side::Buy, 10_000_000, limit, &snap, OrderType::FAK).expect("quote");
        assert_eq!(quote.side, Side::Buy);
        assert_eq!(quote.price_micro, 690_000);
        assert_eq!(quote.maker_micro, 10_000_000);
        assert!(quote.size_micro >= snap.min_size_micro);
    }

    #[test]
    fn trade_limit_rejects_sell_when_tick_rounding_breaks_min_price() {
        let snap = snapshot(Some(700_000), Some(695_000));
        let err = choose_trade_limit(Side::Sell, true, 691_000, 691_000, &snap)
            .expect_err("tick rounding should fall below min price");
        assert!(matches!(err, DispatchResponse::Error { code: -3, .. }));
    }

    #[test]
    fn local_policy_defaults_to_disabled() {
        let policy: PolymarketVenueConfig = toml::from_str("").unwrap();
        let checks = evaluate_local_polymarket_order(&policy, &policy_ctx());
        assert!(local_policy_has_deny(&checks));
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.enabled" && check.outcome == LocalPolicyOutcome::Deny
        }));
    }

    #[test]
    fn local_policy_parses_decimal_caps() {
        let policy: PolymarketVenueConfig = toml::from_str(
            r#"
enabled = true
max_order_usd = "10"
max_daily_usd = "25.5"
require_flag_above_usd = 5
max_price = "0.75"
allow_neg_risk = false
denied_slugs = ["blocked-market"]
"#,
        )
        .unwrap();
        assert!(policy.enabled);
        assert_eq!(policy.max_order_usd, Some(10_000_000));
        assert_eq!(policy.max_daily_usd, Some(25_500_000));
        assert_eq!(policy.require_flag_above_usd, Some(5_000_000));
        assert_eq!(policy.max_price, Some(750_000));
        assert!(!policy.allow_neg_risk);
        assert!(policy.denied_slugs.contains("blocked-market"));

        let float_policy: PolymarketVenueConfig =
            toml::from_str("enabled = true\nmax_price = 0.1\n").unwrap();
        assert_eq!(float_policy.max_price, Some(100_000));
    }

    #[test]
    fn local_policy_daily_cap_fails_closed_when_receipts_unknown() {
        let policy: PolymarketVenueConfig = toml::from_str(
            r#"
enabled = true
max_daily_usd = "100"
"#,
        )
        .unwrap();
        let mut ctx = policy_ctx();
        ctx.receipt_store_readable = false;
        let checks = evaluate_local_polymarket_order(&policy, &ctx);
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.max_daily_usd" && check.outcome == LocalPolicyOutcome::Deny
        }));

        ctx.receipt_store_readable = true;
        ctx.daily_posted_microusd = None;
        let checks = evaluate_local_polymarket_order(&policy, &ctx);
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.max_daily_usd" && check.outcome == LocalPolicyOutcome::Deny
        }));
    }

    #[test]
    fn prepared_onboarding_batch_expires_at_its_sealed_deadline() {
        let prepared = PreparedSigning::new(
            "onboard_approvals",
            "polymarket.onboard",
            Address::ZERO,
            Vec::new(),
            alloy::primitives::B256::ZERO,
            serde_json::json!({"deadline": 500}),
        );
        assert!(
            !crate::infra_parts::relayer::prepared_relayer_batch_expired(&prepared, 499).unwrap()
        );
        assert!(
            crate::infra_parts::relayer::prepared_relayer_batch_expired(&prepared, 500).unwrap()
        );
    }
}
