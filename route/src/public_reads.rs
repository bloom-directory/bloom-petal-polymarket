use crate::*;

use crate::bloom_petal_sdk::DispatchResponse;
use crate::polymarket::{Market, OrderBook, Position, Result, Trade, validate_wallet_name};
use alloy::primitives::Address;
pub(crate) fn parity_json() -> DispatchResponse {
    read_json_value(&serde_json::json!({
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
}

pub(crate) fn market_by_slug(slug: &str) -> Result<Market, DispatchResponse> {
    get_json(&format!("{GAMMA}/markets/slug/{slug}"))
}

pub(crate) fn market_json(slug: &str) -> DispatchResponse {
    match market_by_slug(slug) {
        Ok(market) => read_json_value(&market),
        Err(resp) => resp,
    }
}

pub(crate) fn market_book_json(slug: &str) -> DispatchResponse {
    let market = match market_by_slug(slug) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    let Some(token_id) = market.yes_token_id() else {
        return error(-4, "market has no YES token id");
    };
    match get_json::<OrderBook>(&url_with_query(
        &format!("{CLOB}/book"),
        &[("token_id", token_id)],
    )) {
        Ok(book) => read_json_value(&book),
        Err(resp) => resp,
    }
}

pub(crate) fn market_prices_json(slug: &str) -> DispatchResponse {
    let market = match market_by_slug(slug) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    let Some(token_id) = market.yes_token_id() else {
        return error(-4, "market has no YES token id");
    };
    let midpoint = match get_json::<serde_json::Value>(&url_with_query(
        &format!("{CLOB}/midpoint"),
        &[("token_id", token_id)],
    )) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let spread = match get_json::<serde_json::Value>(&url_with_query(
        &format!("{CLOB}/spread"),
        &[("token_id", token_id)],
    )) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let best_buy = match get_json::<serde_json::Value>(&url_with_query(
        &format!("{CLOB}/price"),
        &[("token_id", token_id), ("side", "BUY")],
    )) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    read_json_value(&serde_json::json!({
        "token_id": token_id,
        "midpoint": midpoint,
        "spread": spread,
        "best_buy": best_buy,
    }))
}

pub(crate) fn search_results(query: &str) -> DispatchResponse {
    let query = query.replace('+', " ");
    match get_json::<serde_json::Value>(&url_with_query(
        &format!("{GAMMA}/public-search"),
        &[("q", &query)],
    )) {
        Ok(value) => read_json_value(&value),
        Err(resp) => resp,
    }
}

pub(crate) fn position_user(segment: &str) -> Result<String, DispatchResponse> {
    if (segment.starts_with("0x") || segment.starts_with("0X"))
        && let Ok(address) = segment.parse::<Address>()
    {
        return Ok(address.to_checksum(None));
    }
    wallet_address(segment).map(|address| address.to_checksum(None))
}

pub(crate) fn positions_json(user: &str) -> DispatchResponse {
    let user = match position_user(user) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match get_json::<Vec<Position>>(&url_with_query(
        &format!("{DATA}/positions"),
        &[("user", &user)],
    )) {
        Ok(value) => read_json_value(&value),
        Err(resp) => resp,
    }
}

pub(crate) fn position_trades_json(user: &str) -> DispatchResponse {
    let user = match position_user(user) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match get_json::<Vec<Trade>>(&url_with_query(
        &format!("{DATA}/trades"),
        &[("user", &user)],
    )) {
        Ok(value) => read_json_value(&value),
        Err(resp) => resp,
    }
}

pub(crate) fn position_activity_json(user: &str) -> DispatchResponse {
    let user = match position_user(user) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match get_json::<serde_json::Value>(&url_with_query(
        &format!("{DATA}/activity"),
        &[("user", &user)],
    )) {
        Ok(value) => read_json_value(&value),
        Err(resp) => resp,
    }
}

pub(crate) fn onboard_begin_hint() -> DispatchResponse {
    DispatchResponse::Read(BEGIN_HINT.into())
}

pub(crate) fn onboard_status_json(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let status = match wallet_address(wallet) {
        Ok(owner) => match local_status_for_wallet(wallet, owner) {
            Ok(status) => status,
            Err(resp) => return resp,
        },
        Err(_) => serde_json::json!({
            "wallet": wallet,
            "stage": "not_started",
            "running": false,
            "tradeable": false,
            "message": "write begin to mint or derive CLOB credentials"
        }),
    };
    read_json_value(&status)
}

pub(crate) fn onboard_plan_md(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    DispatchResponse::Read(render_onboard_plan(wallet).into_bytes())
}

pub(crate) fn onboard_approvals_json(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    match wallet_address(wallet) {
        Ok(owner) => read_json_value(&approval_preview(wallet, owner)),
        Err(resp) => resp,
    }
}

pub(crate) fn account_portfolio_json(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    match clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    ) {
        Ok(clob_balance_allowance) => {
            let status = match local_status_for_wallet(wallet, owner) {
                Ok(status) => status,
                Err(resp) => return resp,
            };
            read_json_value(&serde_json::json!({
                "wallet": wallet,
                "owner": format!("{owner:#x}"),
                "credentials_present": true,
                "clob_balance_allowance": clob_balance_allowance,
                "deposit_wallet": status
                    .get("deposit_wallet")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                "onboarding_state": {
                    "stage": status.get("stage").cloned().unwrap_or(serde_json::Value::Null),
                    "creds_present": status.get("creds_present").cloned().unwrap_or(serde_json::Value::Bool(true)),
                    "tradeable": status.get("tradeable").cloned().unwrap_or(serde_json::Value::Bool(false))
                }
            }))
        }
        Err(resp) => resp,
    }
}

pub(crate) fn account_orders_json(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    match clob_l2_get_json(owner, &creds, "/data/orders", &[]) {
        Ok(orders) => read_json_value(&orders),
        Err(resp) => resp,
    }
}
pub(crate) fn list_market_slugs() -> Result<Vec<String>, DispatchResponse> {
    let url = url_with_query(
        &format!("{GAMMA}/markets"),
        &[
            ("closed", "false"),
            ("limit", &MARKETS_LIST_LIMIT.to_string()),
            ("order", "volumeNum"),
            ("ascending", "false"),
        ],
    );
    let markets: Vec<Market> = get_json(&url)?;
    Ok(markets
        .into_iter()
        .filter_map(|market| (!market.slug.is_empty()).then_some(market.slug))
        .collect())
}
