#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]

//! Local Polymarket handler petal.
//!
//! This petal owns `apps/polymarket/` directly. Public market/account reads go
//! through the v2 `bloom:http` import; staged local state goes through the
//! v2 private store import. It intentionally does not call the legacy native
//! `polymarket/` VFS handler.

wit_bindgen::generate!({
    path: "wit",
    world: "route-file",
    generate_all
});

mod selected_route {
    include!(env!("BLOOM_ROUTE_RS"));
}

mod app_types;
mod constants;
mod framework;
mod fund_flow;
mod host;
mod infra_parts {
    pub(crate) mod clob_l2;
    pub(crate) mod credentials;
    pub(crate) mod host_calls;
    pub(crate) mod http;
    pub(crate) mod lists;
    pub(crate) mod reconcile;
    pub(crate) mod relayer;
    pub(crate) mod render;
    pub(crate) mod store;
    pub(crate) mod util;
}
mod onboarding;
mod polymarket;
mod public_reads;
mod trade_flow_parts {
    pub(crate) mod chain;
    pub(crate) mod draft;
    pub(crate) mod policy;
    pub(crate) mod posting;
    pub(crate) mod pricing;
    pub(crate) mod revalidate;
    pub(crate) mod storage;
}

#[cfg(test)]
mod app_tests;

pub(crate) use crate::bloom::route::types::EntryKind;
#[allow(unused_imports)]
pub(crate) use app_types::{
    FundNewRequest, GeoblockStatus, LocalPolicyCheck, LocalPolicyOutcome, LocalPolicySide,
    LocalPolymarketOrderCtx, LocalPolymarketPolicy, LocalWalletPolicy, StoreFundSession,
    StoreTradeDraft, StoreTradeReceipt, StoreTradeReceiptPolicy, TradeCancelRequest,
    TradeNewRequest, TradePostRequest, TradeRevalidateRequest, TradeSnapshot, default_slippage_bps,
    default_true,
};
#[allow(unused_imports)]
pub(crate) use constants::{
    ACCOUNT_FILES, BATCH_DEADLINE_SECS, BEGIN_HINT, CLOB, CLOB_AUTH_NONCE, DATA, DRAFT_FILES,
    DRAFT_WRITABLE_FILES, FUND_FILES, FUND_NEW_HINT, GAMMA, MARKET_FILES, MARKETS_LIST_LIMIT,
    MAX_CHAIN_METHOD_BYTES, MAX_CHAIN_READ_BYTES, MAX_HTTP_BYTES, MAX_LIST_BYTES, MAX_POLICY_BYTES,
    MAX_STORE_BYTES, META_FILES, ONBOARD_FILES, ONBOARD_POLL_INTERVAL_SECS,
    ONBOARD_POLL_TIMEOUT_SECS, POLYMARKET_WEB, POSITION_FILES, RECEIPT_FILES,
    RECEIPT_WRITABLE_FILES, RELAYER, ROOT_DIRS, TRADE_CANCEL_HINT, TRADE_LOCK_STALE_MS,
    TRADE_NEW_HINT, TRADE_POST_HINT, TRADE_REVALIDATE_HINT,
};
#[allow(unused_imports)]
pub(crate) use framework::{
    RouteChild, RouteFileKind, RouteSpec, account_read_spec, chain_read_spec,
    current_route_canonical_path, current_route_path, dir, dir_names, dirs, entry_name, error,
    file, files, framework_entry, framework_fallible_list, framework_list, framework_lookup,
    framework_metadata, framework_read, framework_write, http_dir_spec, http_read_spec,
    is_safe_segment, metadata_path, param, result_dirs, route_error, route_generated_param,
    route_invalid, route_param, route_relative, route_segment, split, static_dir_spec,
    static_read_spec, store_dir_spec, store_read_spec, wallet_http_read_spec, writable, write_spec,
};
#[allow(unused_imports)]
pub(crate) use fund_flow::{create_fund_request, load_fund_session};
pub(crate) use host::bloom_petal_sdk;
#[allow(unused_imports)]
pub(crate) use host::bloom_petal_sdk::DispatchResponse;
#[allow(unused_imports)]
pub(crate) use infra_parts::clob_l2::{clob_l2_delete_json, clob_l2_get_json, clob_l2_post_json};
#[allow(unused_imports)]
pub(crate) use infra_parts::credentials::{
    delete_builder_credentials, ensure_builder_credentials, load_builder_credentials, load_creds,
    save_builder_credentials,
};
#[allow(unused_imports)]
pub(crate) use infra_parts::host_calls::{http, wallet_address};
#[allow(unused_imports)]
pub(crate) use infra_parts::http::{clob_auth_request, get_json};
#[allow(unused_imports)]
pub(crate) use infra_parts::lists::{
    next_id, safe_wallet_names, store_ids, store_wallets, vfs_wallets_or_store,
};
#[allow(unused_imports)]
pub(crate) use infra_parts::reconcile::{
    address_strings_equal, blake3_hex, clob_cancel_confirmed, clob_order_field_strings,
    clob_order_field_u64s, clob_order_fields, clob_reconciled_public_summary,
    clob_response_filled_size_micro, clob_response_order_id, clob_response_public_summary,
    clob_response_status, clob_side_value_matches, clob_status_excluded_from_daily_cap,
    find_matching_open_order, open_order_matches_draft, reconcile_ambiguous_post,
};
#[allow(unused_imports)]
pub(crate) use infra_parts::relayer::{
    LocalRelayerTx, RelayerHttpError, builder_headers, builder_hmac_signature,
    dispatch_error_message, onboard_in_flight_deadline_ms, parse_json_u64,
    parse_relayer_submit_response, parse_relayer_transaction_response, relayer_batch_body,
    relayer_get_json, relayer_http_error, relayer_poll_confirmed, relayer_submit,
    relayer_submit_with_builder_repair, relayer_transaction, relayer_tx_id_matches,
    relayer_wallet_nonce, sign_hash_hex,
};
#[allow(unused_imports)]
pub(crate) use infra_parts::render::read_json_value;
#[allow(unused_imports)]
pub(crate) use infra_parts::store::{
    StoreTradeLock, acquire_trade_lock, append_trade_audit, read_store, store_get, store_put_json,
    store_trade_receipt, trade_lock_body, trade_lock_stale_bytes,
};
#[allow(unused_imports)]
pub(crate) use infra_parts::util::{
    now_millis, now_secs, polymarket_error, sdk_error, sdk_error_with_context, url_with_query,
    validate_relative_path,
};
#[allow(unused_imports)]
pub(crate) use onboarding::{
    LiveOnboardStatus, OnboardStatusExtra, begin_onboarding, check_geoblock,
    fundable_deposit_wallet, fundable_deposit_wallet_from_status, local_onboard_status,
    local_onboard_status_with_live_deposit, local_status_for_wallet, persist_onboard_failure,
    persist_onboard_status, preserve_onboard_metadata, refreshed_live_onboard_status,
    run_onboard_stages, stored_status_for_wallet, tradeable_deposit_wallet,
};
#[allow(unused_imports)]
pub(crate) use polymarket::POLYGON;
pub(crate) use polymarket::{
    PolymarketError, Result, eip712, order, order_store, signer, trade, types,
    validate_wallet_name, wallet,
};
#[allow(unused_imports)]
pub(crate) use public_reads::{market_by_slug, position_user};
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::chain::{
    allowance_floor, chain_method_nonce, parse_json_u256, predict_deposit_wallet,
    read_chain_address, read_chain_ctf_approval, read_chain_ctf_balance,
    read_chain_deposit_wallet_deployed, read_chain_erc20_allowance, read_chain_erc20_balance,
    read_chain_method, read_chain_v2_approvals, read_clob_collateral_sync, read_decoded_u256,
    v2_spenders,
};
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::draft::create_trade_draft;
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::policy::{
    audited_receipt_ids_since, daily_posted_microusd, enable_trade_posting,
    evaluate_local_polymarket_order, local_policy_check, local_policy_has_deny,
    local_policy_has_warn, local_policy_list_check, parse_api_float_micro, parse_clob_raw_micro,
    parse_json_f64_micro, position_size_micro, verify_sell_preflight, wallet_policy,
};
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::posting::{
    cancel_trade_receipt, mark_trade_draft_cancelled, post_trade_draft,
};
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::pricing::{
    best_price, build_trade_quote, choose_trade_limit, trade_policy_check, trade_snapshot,
};
#[allow(unused_imports)]
pub(crate) use trade_flow_parts::revalidate::{
    refresh_trade_post_inputs, revalidate_trade_draft, review_intent_matches_draft,
};
pub(crate) use trade_flow_parts::storage::load_trade_draft;

#[cfg(not(test))]
use selected_route::Route;

#[cfg(not(test))]
export!(Route);
