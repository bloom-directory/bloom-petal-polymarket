#[cfg(test)]
mod tests {
    use crate::app_types::{StoreTradeDraft, TradeSnapshot};
    use crate::infra_parts::util::{url_with_query, validate_relative_path};
    use crate::polymarket::fees::MarketFee;
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
            fees_enabled: Some(false.into()),
            fee_type: None,
            fee_schedule: None,
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
            fee: MarketFee::Disabled,
        }
    }

    fn policy_ctx() -> LocalPolymarketOrderCtx {
        LocalPolymarketOrderCtx {
            slug: "example".into(),
            condition_id: "0xabc".into(),
            side: LocalPolicySide::Buy,
            amount_microusd: 10_000_000,
            fee_cap_microusd: 0,
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
            fee: Some(MarketFee::Disabled),
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

        assert_eq!(routes.len(), 109);
        assert!(routes.iter().any(|path| path.ends_with("$index.rs")));
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

    /// `crypto_fees_v2`: rate 0.07, exponent 1, takers only.
    fn crypto_fee() -> MarketFee {
        MarketFee::Enabled {
            fee_type: Some("crypto_fees_v2".into()),
            rate_ppm: 70_000,
            taker_only: true,
        }
    }

    fn unbounded_fee() -> MarketFee {
        MarketFee::Unbounded {
            fee_type: Some("crypto_fees_v2".into()),
            reason: "feeSchedule exponent 2 is not 1, the only fee curve this Petal bounds".into(),
        }
    }

    /// The live order: BUY 5 @ 0.40, 2,000,000 pUSD, tick 0.01.
    fn fee_enabled_buy() -> StoreTradeDraft {
        StoreTradeDraft {
            amount_micro: 2_000_000,
            price_bound_micro: 400_000,
            limit_price_micro: 400_000,
            size_micro: 5_000_000,
            maker_micro: 2_000_000,
            taker_micro: 5_000_000,
            fee: Some(crypto_fee()),
            ..draft()
        }
    }

    fn check(checks: &[LocalPolicyCheck], rule: &str) -> LocalPolicyOutcome {
        checks
            .iter()
            .find(|check| check.rule == rule)
            .unwrap_or_else(|| panic!("no {rule} check"))
            .outcome
    }

    /// The order `post_trade_draft` builds and signs for `draft`.
    fn signed_order(draft: &StoreTradeDraft) -> crate::polymarket::order::Order {
        use crate::polymarket::order::{LimitQuote, OrderParams, SIG_TYPE_POLY_1271, build_order};
        build_order(
            &OrderParams {
                token_id: draft.token_id.parse().unwrap(),
                maker: Address::repeat_byte(0x22),
                quote: LimitQuote {
                    side: draft.side,
                    price_micro: draft.limit_price_micro,
                    size_micro: draft.size_micro,
                    maker_micro: draft.maker_micro,
                    taker_micro: draft.taker_micro,
                },
                builder_code: None,
                signature_type: SIG_TYPE_POLY_1271,
            },
            1_700_000_000_000,
        )
    }

    fn pusd_debit(amount: &str) -> petal::ClaimDebit {
        petal::ClaimDebit {
            asset: crate::polymarket::claim::pusd_asset("polygon"),
            amount: amount.into(),
        }
    }

    /// The live case end to end, without the host: Gamma's fee-enabled market
    /// and the CLOB book become the snapshot, the snapshot's fee lands on the
    /// draft, and the signed BUY declares makerAmount plus the fee bound.
    #[test]
    fn the_live_fee_flows_from_gamma_to_the_signed_claim() {
        use crate::polymarket::OrderBook;
        use crate::polymarket::fees::tests::FEE_ENABLED_MARKET;
        use crate::trade_flow_parts::pricing::snapshot_from_book;

        let market: Market = serde_json::from_str(FEE_ENABLED_MARKET).unwrap();
        let book: OrderBook = serde_json::from_value(serde_json::json!({
            "market": "0xabc",
            "asset_id": "1",
            "tick_size": "0.01",
            "min_order_size": "5",
            "neg_risk": false,
            "bids": [{"price": "0.39", "size": "100"}],
            "asks": [{"price": "0.4", "size": "100"}],
        }))
        .unwrap();
        let snapshot = snapshot_from_book(market, "YES", "1".into(), book).unwrap();
        assert_eq!(snapshot.fee, crypto_fee());
        assert_eq!(snapshot.tick_micro, 10_000);

        let quote =
            build_trade_quote(Side::Buy, 2_000_000, 400_000, &snapshot, OrderType::GTC).unwrap();
        assert_eq!(quote.maker_micro, 2_000_000);
        // A draft quoted earlier on a fee-free market picks up the live fee.
        let mut draft = StoreTradeDraft {
            token_id: "1".into(),
            order_type: OrderType::GTC,
            amount_micro: 2_000_000,
            fee: Some(MarketFee::Disabled),
            ..draft()
        };
        draft.requote(&quote, &snapshot, 7);
        assert_eq!(draft.fee, Some(crypto_fee()));
        assert_eq!(draft.maker_micro, 2_000_000);
        assert_eq!(draft.limit_price_micro, 400_000);
        assert_eq!(draft.book_snapshot_secs, 7);

        let order = signed_order(&draft);
        let effects = draft.order_effects("polygon", &order).unwrap();
        // 2,000,000 + ceil(2,000,000 × 0.07) + 1,000: covers the 2,084,000
        // the exchange pulled, at any fill price and any tick.
        assert_eq!(effects.debits(), &[pusd_debit("2141000")]);
        assert_eq!(draft.fee_cap_micro().unwrap(), 141_000);

        // A fee-free market declares the principal alone.
        let free = StoreTradeDraft {
            fee: Some(MarketFee::Disabled),
            ..draft.clone()
        };
        let effects = free.order_effects("polygon", &order).unwrap();
        assert_eq!(effects.debits(), &[pusd_debit("2000000")]);
    }

    #[test]
    fn a_snapshot_records_a_fee_it_cannot_bound_without_failing() {
        use crate::polymarket::OrderBook;
        use crate::polymarket::fees::tests::FEE_ENABLED_MARKET;
        use crate::trade_flow_parts::pricing::snapshot_from_book;

        let mut value: serde_json::Value = serde_json::from_str(FEE_ENABLED_MARKET).unwrap();
        value["feeSchedule"]["exponent"] = 2.into();
        let book: OrderBook = serde_json::from_value(serde_json::json!({
            "market": "0xabc",
            "asset_id": "1",
            "tick_size": "0.01",
        }))
        .unwrap();
        let snapshot = snapshot_from_book(
            serde_json::from_value(value).unwrap(),
            "YES",
            "1".into(),
            book,
        )
        .unwrap();
        assert!(matches!(snapshot.fee, MarketFee::Unbounded { .. }));
    }

    /// A `feesEnabled` that Gamma stops sending as a boolean still quotes the
    /// market: a SELL is quoted and its claim declared, and only a BUY, whose
    /// fee cannot be bounded, is refused.
    #[test]
    fn a_retyped_fees_enabled_refuses_only_a_buy() {
        use crate::polymarket::OrderBook;
        use crate::polymarket::fees::tests::FEE_ENABLED_MARKET;
        use crate::trade_flow_parts::pricing::snapshot_from_book;

        let mut value: serde_json::Value = serde_json::from_str(FEE_ENABLED_MARKET).unwrap();
        value["feesEnabled"] = "true".into();
        let book: OrderBook = serde_json::from_value(serde_json::json!({
            "market": "0xabc",
            "asset_id": "1",
            "tick_size": "0.01",
            "min_order_size": "5",
            "neg_risk": false,
            "bids": [{"price": "0.39", "size": "100"}],
            "asks": [{"price": "0.4", "size": "100"}],
        }))
        .unwrap();
        let snapshot = snapshot_from_book(
            serde_json::from_value(value).unwrap(),
            "YES",
            "1".into(),
            book,
        )
        .unwrap();
        assert!(matches!(snapshot.fee, MarketFee::Unbounded { .. }));

        let quote =
            build_trade_quote(Side::Sell, 5_000_000, 390_000, &snapshot, OrderType::GTC).unwrap();
        let mut sell = StoreTradeDraft {
            side: Side::Sell,
            token_id: "1".into(),
            order_type: OrderType::GTC,
            ..draft()
        };
        sell.requote(&quote, &snapshot, 7);
        assert_eq!(sell.fee_cap_micro().unwrap(), 0);
        let effects = sell.order_effects("polygon", &signed_order(&sell)).unwrap();
        assert_eq!(effects.debits().len(), 1);
        assert_ne!(effects.debits()[0].asset, pusd_debit("0").asset);

        let quote =
            build_trade_quote(Side::Buy, 2_000_000, 400_000, &snapshot, OrderType::GTC).unwrap();
        let mut buy = StoreTradeDraft {
            token_id: "1".into(),
            order_type: OrderType::GTC,
            ..draft()
        };
        buy.requote(&quote, &snapshot, 7);
        let err = buy.fee_cap_micro().unwrap_err().to_string();
        assert!(err.contains("feesEnabled"), "{err}");
        assert!(err.contains("refusing to sign a BUY"), "{err}");
        assert!(buy.order_effects("polygon", &signed_order(&buy)).is_err());
    }

    /// Posting refuses before signing when the prepared order is not the
    /// reviewed one, or the draft has no recorded fee.
    #[test]
    fn a_claim_is_only_declared_for_the_reviewed_order_and_fee() {
        let draft = fee_enabled_buy();
        let mut order = signed_order(&draft);
        order.makerAmount = alloy::primitives::U256::from(1_000_000u64);
        let err = draft
            .order_effects("polygon", &order)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match the reviewed draft"), "{err}");

        let old = StoreTradeDraft {
            fee: None,
            ..fee_enabled_buy()
        };
        let err = old
            .order_effects("polygon", &signed_order(&old))
            .unwrap_err()
            .to_string();
        assert!(err.contains("write revalidate"), "{err}");

        let unbounded = StoreTradeDraft {
            fee: Some(unbounded_fee()),
            ..fee_enabled_buy()
        };
        let err = unbounded
            .order_effects("polygon", &signed_order(&unbounded))
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing to sign a BUY"), "{err}");
    }

    /// The bound is taken on the signed makerAmount, not the requested amount.
    #[test]
    fn the_fee_bound_is_taken_on_the_maker_amount() {
        let draft = StoreTradeDraft {
            amount_micro: 2_100_000,
            ..fee_enabled_buy()
        };
        assert_eq!(draft.fee_cap_micro().unwrap(), 141_000);
    }

    #[test]
    fn the_fee_bound_counts_against_the_usd_caps() {
        let policy: PolymarketVenueConfig = toml::from_str(
            r#"
enabled = true
max_order_usd = "2.1"
max_daily_usd = "10"
require_flag_above_usd = "2.05"
"#,
        )
        .unwrap();

        // 2.0 alone fits every cap; 2.0 + 0.141 does not.
        let free = StoreTradeDraft {
            fee: Some(MarketFee::Disabled),
            ..fee_enabled_buy()
        };
        let ctx = LocalPolymarketOrderCtx::for_draft(&free, true, Some(7_900_000)).unwrap();
        assert_eq!(ctx.fee_cap_microusd, 0);
        let checks = evaluate_local_polymarket_order(&policy, &ctx);
        assert_eq!(
            check(&checks, "polymarket.max_order_usd"),
            LocalPolicyOutcome::Pass
        );
        assert_eq!(
            check(&checks, "polymarket.max_daily_usd"),
            LocalPolicyOutcome::Pass
        );
        assert!(!local_policy_has_warn(&checks));

        let buy = fee_enabled_buy();
        let ctx = LocalPolymarketOrderCtx::for_draft(&buy, true, Some(7_900_000)).unwrap();
        assert_eq!(ctx.amount_microusd, 2_000_000);
        assert_eq!(ctx.fee_cap_microusd, 141_000);
        let checks = evaluate_local_polymarket_order(&policy, &ctx);
        assert_eq!(
            check(&checks, "polymarket.max_order_usd"),
            LocalPolicyOutcome::Deny
        );
        assert_eq!(
            check(&checks, "polymarket.max_daily_usd"),
            LocalPolicyOutcome::Deny
        );
        assert_eq!(
            check(&checks, "polymarket.require_flag_above_usd"),
            LocalPolicyOutcome::Warn
        );
        let message = &checks
            .iter()
            .find(|check| check.rule == "polymarket.max_order_usd")
            .unwrap()
            .message;
        assert!(
            message.contains("2.141 (amount 2 + fee bound 0.141)"),
            "{message}"
        );

        // A BUY whose fee cannot be bounded has no policy view at all.
        let unbounded = StoreTradeDraft {
            fee: Some(unbounded_fee()),
            ..fee_enabled_buy()
        };
        assert!(LocalPolymarketOrderCtx::for_draft(&unbounded, true, Some(0)).is_err());
    }

    #[test]
    fn receipts_carry_the_fee_bound_into_the_daily_cap() {
        use crate::trade_flow_parts::policy::receipt_daily_exposure_microusd;

        let funder = Address::repeat_byte(0x22).to_checksum(None);
        let mut receipt =
            StoreTradeReceipt::for_draft(&fee_enabled_buy(), funder.clone(), 42, "h".into())
                .unwrap();
        assert_eq!(receipt.fee_cap_microusd, Some(141_000));
        assert_eq!(receipt.fee, Some(crypto_fee()));
        assert_eq!(receipt.funder.as_deref(), Some(funder.as_str()));
        assert_eq!(receipt.review_intent_hash.as_deref(), Some("h"));
        assert_eq!(receipt.clob_status, "ambiguous");
        receipt.clob_status = "matched".into();
        receipt.posted_ms = 1_000;
        let stored = serde_json::to_value(&receipt).unwrap();
        assert_eq!(stored["fee"]["kind"], "enabled");
        assert_eq!(stored["fee"]["rate_ppm"], 70_000);
        assert_eq!(stored["fee_cap_microusd"], 141_000);
        let policy: StoreTradeReceiptPolicy = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(receipt_daily_exposure_microusd(&policy, 1_000), 2_141_000);
        // Outside the day, rejected, or a SELL: nothing.
        assert_eq!(receipt_daily_exposure_microusd(&policy, 1_001), 0);
        let rejected = StoreTradeReceiptPolicy {
            clob_status: "rejected".into(),
            ..policy.clone()
        };
        assert_eq!(receipt_daily_exposure_microusd(&rejected, 0), 0);
        let sell = StoreTradeReceiptPolicy {
            side: Side::Sell,
            ..policy
        };
        assert_eq!(receipt_daily_exposure_microusd(&sell, 0), 0);

        // A receipt written before fees were bounded counts the bound at the
        // highest accepted rate (2,000,000 × 0.25 + 1,000), even after the
        // cancel path rewrites it.
        let mut old = stored;
        let object = old.as_object_mut().unwrap();
        object.remove("fee_cap_microusd");
        object.remove("fee");
        let rewritten: StoreTradeReceipt = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(rewritten.fee, None);
        for value in [old, serde_json::to_value(&rewritten).unwrap()] {
            let policy: StoreTradeReceiptPolicy = serde_json::from_value(value).unwrap();
            assert_eq!(policy.fee_cap_microusd, None);
            assert_eq!(receipt_daily_exposure_microusd(&policy, 0), 2_501_000);
        }

        // No receipt is built for a BUY whose fee is unbounded or unrecorded.
        for fee in [Some(unbounded_fee()), None] {
            let draft = StoreTradeDraft {
                fee,
                ..fee_enabled_buy()
            };
            assert!(StoreTradeReceipt::for_draft(&draft, funder.clone(), 42, "h".into()).is_err());
        }
    }

    /// The prepared approval record and the receipt show a BUY's fee bound
    /// with the note on what enforces it. Records an earlier build wrote
    /// without the note still load, and keep their JSON.
    #[test]
    fn the_fee_bound_note_is_shown_wherever_the_bound_is() {
        use crate::polymarket::fees::FEE_BOUND_NOTE;
        use crate::polymarket::order::poly1271_digest;
        use crate::trade_flow_parts::posting::prepared_order_signing;

        let buy = fee_enabled_buy();
        let order = signed_order(&buy);
        let owner = Address::repeat_byte(0x11);
        let funder = Address::repeat_byte(0x22);
        let prepared = prepared_order_signing(
            &buy,
            &order,
            owner,
            funder,
            137,
            buy.fee_cap_micro().unwrap(),
            "h",
            false,
        );
        assert_eq!(prepared.preimage["fee"], serde_json::json!(buy.fee));
        assert_eq!(prepared.preimage["fee_cap"], "0.141");
        assert_eq!(prepared.preimage["fee_note"], FEE_BOUND_NOTE);
        assert_eq!(prepared.preimage["draft_id"], buy.id);
        assert_eq!(prepared.preimage["review_intent_hash"], "h");
        assert_eq!(
            prepared.hash().unwrap(),
            poly1271_digest(&order, 137, buy.neg_risk)
        );
        // Posting reuses a stored prepared record as it is, so one without
        // the note keeps the bytes its owner approval is bound to.
        let mut earlier = prepared.clone();
        earlier.preimage.as_object_mut().unwrap().remove("fee_note");
        let stored = serde_json::to_vec(&earlier).unwrap();
        let loaded: PreparedSigning = serde_json::from_slice(&stored).unwrap();
        assert_eq!(loaded, earlier);
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), stored);

        let receipt =
            StoreTradeReceipt::for_draft(&buy, funder.to_checksum(None), 42, "h".into()).unwrap();
        assert_eq!(receipt.fee_note.as_deref(), Some(FEE_BOUND_NOTE));
        let stored = serde_json::to_value(&receipt).unwrap();
        assert_eq!(stored["fee_cap_microusd"], 141_000);
        assert_eq!(stored["fee_note"], FEE_BOUND_NOTE);
        // A receipt from an earlier build has no note. It loads, and a
        // rewrite (the cancel path's) leaves the note out.
        let mut earlier = stored;
        earlier.as_object_mut().unwrap().remove("fee_note");
        let loaded: StoreTradeReceipt = serde_json::from_value(earlier.clone()).unwrap();
        assert_eq!(loaded.fee_note, None);
        assert_eq!(serde_json::to_value(&loaded).unwrap(), earlier);
        let policy: StoreTradeReceiptPolicy = serde_json::from_value(earlier).unwrap();
        assert_eq!(policy.fee_cap_microusd, Some(141_000));
    }

    /// Once the CLOB answers, `post_attempt.json` records the outcome in
    /// place of the `signed` record.
    #[test]
    fn the_post_attempt_records_the_clob_answer() {
        use crate::trade_flow_parts::posting::answered_post_attempt;

        let owner = Address::repeat_byte(0x11);
        let funder = Address::repeat_byte(0x22).to_checksum(None);
        let unposted =
            StoreTradeReceipt::for_draft(&fee_enabled_buy(), funder.clone(), 42, "h".into())
                .unwrap();
        let posted = StoreTradeReceipt {
            clob_order_id: Some("0xorder".into()),
            clob_status: "live".into(),
            filled_size_micro: Some(0),
            posted_ms: 1_000,
            ..unposted.clone()
        };
        assert_eq!(
            answered_post_attempt(&posted, owner, "body", "posted", None),
            serde_json::json!({
                "wallet": "alice",
                "draft_id": "0001",
                "owner": owner.to_checksum(None),
                "funder": funder,
                "salt": 42,
                "review_intent_hash": "h",
                "order_body_blake3": "body",
                "posted_ms": 1_000,
                "status": "posted",
                "clob_order_id": "0xorder",
                "clob_status": "live",
            })
        );

        let rejected = StoreTradeReceipt {
            clob_status: "rejected".into(),
            posted_ms: 2_000,
            ..unposted
        };
        let attempt = answered_post_attempt(
            &rejected,
            owner,
            "body",
            "rejected",
            Some("CLOB post error (status 400): not enough balance"),
        );
        assert_eq!(attempt["status"], "rejected");
        assert_eq!(attempt["clob_status"], "rejected");
        assert_eq!(attempt["clob_order_id"], serde_json::Value::Null);
        assert_eq!(
            attempt["error"],
            "CLOB post error (status 400): not enough balance"
        );
        assert_eq!(attempt["posted_ms"], 2_000);
    }

    mod plan_route {
        include!("../files/trade/[wallet]/[index]/drafts/[id]/plan.md.rs");
    }

    /// plan.md says where the draft stands, from the state posting stores on
    /// it, rather than always reporting signing and posting as pending.
    #[test]
    fn the_plan_shows_the_post_outcome() {
        let plan = |edit: &dyn Fn(&mut StoreTradeDraft)| {
            let mut draft = fee_enabled_buy();
            edit(&mut draft);
            plan_route::plan_md(&draft)
        };
        let pending = "Signing and posting are still pending.";

        for status in ["review", "revalidated"] {
            let plan = plan(&|draft| draft.status = status.into());
            assert!(plan.contains(pending), "{status}: {plan}");
        }
        let prepared = plan(&|draft| draft.status = "signing_prepared".into());
        assert!(
            prepared.contains("waits for the owner's approval"),
            "{prepared}"
        );

        let posted = plan(&|draft| {
            draft.status = "posted".into();
            draft.clob_order_id = Some("0xorder".into());
            draft.clob_status = Some("live".into());
        });
        assert!(
            posted.contains("Posted to the CLOB as order 0xorder, CLOB status live."),
            "{posted}"
        );
        assert!(posted.contains("Status: posted"), "{posted}");
        assert!(posted.contains("Fee bound: 0.141 pUSD"), "{posted}");

        let rejected = plan(&|draft| {
            draft.status = "rejected".into();
            draft.clob_status = Some("rejected".into());
            draft.last_error = Some("CLOB post error (status 400): not enough balance".into());
        });
        assert!(
            rejected.contains(
                "Not posted. The CLOB rejected the order: CLOB post error (status 400): not enough balance"
            ),
            "{rejected}"
        );
        // A FAK that did not match is answered, not refused.
        let unmatched = plan(&|draft| {
            draft.status = "rejected".into();
            draft.clob_order_id = Some("0xorder".into());
            draft.clob_status = Some("unmatched".into());
        });
        assert!(
            unmatched.contains("the CLOB answered order 0xorder with status unmatched"),
            "{unmatched}"
        );

        let ambiguous = plan(&|draft| {
            draft.status = "ambiguous".into();
            draft.last_error = Some("post outcome unknown after signing".into());
        });
        assert!(
            ambiguous.contains("its outcome is unknown (post outcome unknown after signing)"),
            "{ambiguous}"
        );
        let cancelled = plan(&|draft| {
            draft.status = "cancelled".into();
            draft.clob_order_id = Some("0xorder".into());
        });
        assert!(
            cancelled.contains("Posted to the CLOB as order 0xorder, then cancelled."),
            "{cancelled}"
        );

        for plan in [prepared, posted, rejected, unmatched, ambiguous, cancelled] {
            assert!(!plan.contains(pending), "{plan}");
        }
    }

    /// A draft stored by an older build has no fee. It still loads, but it
    /// has no fee bound and its old final review cannot be posted.
    #[test]
    fn a_draft_without_fee_data_must_be_revalidated() {
        let mut stored = serde_json::to_value(fee_enabled_buy()).unwrap();
        stored.as_object_mut().unwrap().remove("fee");
        let old: StoreTradeDraft = serde_json::from_value(stored).unwrap();
        assert_eq!(old.fee, None);
        let err = old.fee_cap_micro().unwrap_err().to_string();
        assert!(err.contains("write revalidate"), "{err}");

        let owner = Address::repeat_byte(0x11);
        let funder = Address::repeat_byte(0x22);
        let policy_check = serde_json::json!({"policy_status": "passed"});
        let old_review = serde_json::json!({
            "wallet": old.wallet,
            "draft_id": old.id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "slug": old.slug,
            "condition_id": old.condition_id,
            "outcome": old.outcome,
            "token_id": old.token_id,
            "side": "BUY",
            "order_type": old.order_type.as_str(),
            "limit_price": "0.4",
            "size": "5",
            "maker": "2",
            "taker": "5",
            "neg_risk": false,
            "policy_status": "passed",
        });
        // Not postable while the draft has no fee, even against a review
        // that recorded no fee and a zero bound ...
        assert!(
            review_intent_matches_draft(&old_review, &old, owner, funder, &policy_check, None)
                .is_err()
        );
        let mut feeless_review = old_review.clone();
        feeless_review["fee"] = serde_json::Value::Null;
        feeless_review["fee_cap"] = "0".into();
        let err =
            review_intent_matches_draft(&feeless_review, &old, owner, funder, &policy_check, None)
                .unwrap_err();
        assert!(err.contains("write revalidate"), "{err}");
        // ... nor once post refreshes it from the live market, because the
        // owner never reviewed a fee bound.
        let refreshed = fee_enabled_buy();
        let err = review_intent_matches_draft(
            &old_review,
            &refreshed,
            owner,
            funder,
            &policy_check,
            None,
        )
        .unwrap_err();
        assert!(err.contains("fee_cap"), "{err}");

        // A final review that shows the fee and its bound matches.
        let mut review = old_review;
        review["fee"] = serde_json::json!(refreshed.fee);
        review["fee_cap"] = "0.141".into();
        review_intent_matches_draft(&review, &refreshed, owner, funder, &policy_check, None)
            .unwrap();
        // A changed schedule does not.
        let changed = StoreTradeDraft {
            fee: Some(MarketFee::Enabled {
                fee_type: Some("crypto_fees_v2".into()),
                rate_ppm: 70_000,
                taker_only: false,
            }),
            ..fee_enabled_buy()
        };
        let err =
            review_intent_matches_draft(&review, &changed, owner, funder, &policy_check, None)
                .unwrap_err();
        assert!(err.contains("'fee'"), "{err}");
        // Nor does a changed rate, which changes the bound.
        let pricier = StoreTradeDraft {
            fee: Some(MarketFee::Enabled {
                fee_type: Some("crypto_fees_v2".into()),
                rate_ppm: 75_000,
                taker_only: true,
            }),
            ..fee_enabled_buy()
        };
        let err =
            review_intent_matches_draft(&review, &pricier, owner, funder, &policy_check, None)
                .unwrap_err();
        assert!(err.contains("'fee_cap'"), "{err}");
    }

    /// A SELL's claim debits only its outcome tokens, and Polymarket is
    /// expected to take its fee out of the proceeds (not yet verified on
    /// chain; see #20), so the fee schedule never blocks a SELL.
    #[test]
    fn a_sell_is_not_held_to_the_fee_schedule() {
        let sell = StoreTradeDraft {
            side: Side::Sell,
            fee: Some(unbounded_fee()),
            ..fee_enabled_buy()
        };
        assert_eq!(sell.fee_cap_micro().unwrap(), 0);
        let quoted_by_an_old_build = StoreTradeDraft {
            fee: None,
            ..sell.clone()
        };
        assert_eq!(quoted_by_an_old_build.fee_cap_micro().unwrap(), 0);
        let ctx = LocalPolymarketOrderCtx::for_draft(&sell, true, Some(0)).unwrap();
        assert_eq!(ctx.fee_cap_microusd, 0);
        let effects = sell.order_effects("polygon", &signed_order(&sell)).unwrap();
        assert_eq!(effects.debits().len(), 1);
        assert_ne!(effects.debits()[0].asset, pusd_debit("0").asset);
        let receipt = StoreTradeReceipt::for_draft(&sell, "0x22".into(), 42, "h".into()).unwrap();
        assert_eq!(receipt.fee_cap_microusd, Some(0));

        // A SELL reviewed before fees were recorded, or under a schedule that
        // has since changed, still matches its final review.
        let owner = Address::repeat_byte(0x11);
        let funder = Address::repeat_byte(0x22);
        let preflight = serde_json::json!({"balance": "5"});
        let review = serde_json::json!({
            "wallet": sell.wallet,
            "draft_id": sell.id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "slug": sell.slug,
            "condition_id": sell.condition_id,
            "outcome": sell.outcome,
            "token_id": sell.token_id,
            "side": "SELL",
            "order_type": sell.order_type.as_str(),
            "limit_price": "0.4",
            "size": "5",
            "maker": "2",
            "taker": "5",
            "neg_risk": false,
            "policy_status": "passed",
            "sell_preflight": preflight,
        });
        let policy_check = serde_json::json!({"policy_status": "passed"});
        for fee in [Some(unbounded_fee()), Some(crypto_fee()), None] {
            let sell = StoreTradeDraft {
                fee,
                ..sell.clone()
            };
            review_intent_matches_draft(
                &review,
                &sell,
                owner,
                funder,
                &policy_check,
                Some(&preflight),
            )
            .unwrap();
        }
    }
}
