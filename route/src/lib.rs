#![allow(clippy::too_many_arguments)]
#![cfg_attr(test, allow(dead_code))]
#![recursion_limit = "256"]

//! Local Polymarket handler petal.
//!
//! This petal owns `apps/polymarket/` directly. Public market/account reads go
//! through the v2 `bloom:http` import; staged local state goes through the
//! v2 private store import. It intentionally does not call the legacy native
//! `polymarket/` VFS handler.

use crate::bloom::route::types::EntryKind;

wit_bindgen::generate!({
    path: "wit",
    world: "route-file",
    generate_all
});

struct Route;

impl Guest for Route {
    fn metadata(ctx: Ctx) -> Result<RouteMeta, RouteError> {
        let relative = metadata_path(&ctx.path);
        let kind = match path_kind(&relative) {
            Some(DispatchEntryKind::Dir) => EntryKind::Dir,
            Some(DispatchEntryKind::File | DispatchEntryKind::WritableFile) => EntryKind::File,
            None => return Err(RouteError::NotFound(ctx.path)),
        };
        let writable = matches!(path_kind(&relative), Some(DispatchEntryKind::WritableFile));
        Ok(RouteMeta {
            kind,
            mode: match kind {
                EntryKind::Dir => 0o755,
                EntryKind::File if writable => 0o644,
                EntryKind::File => 0o444,
                EntryKind::Symlink => 0o777,
            },
            cache_ttl_ms: route_cache_ttl_ms(&relative),
            side_effecting_read: false,
            write_async: false,
            description: Some(format!("Polymarket route {relative}")),
            consent_summary: None,
            required_caps: route_required_caps(&relative, writable),
            sign_intent: None,
            executable: false,
        })
    }

    fn lookup(ctx: Ctx) -> Result<Entry, RouteError> {
        match lookup(&ctx.path) {
            DispatchResponse::Lookup(entry) => Ok(route_entry(entry)),
            DispatchResponse::Error { code, message } => Err(route_error(code, message)),
            _ => Err(RouteError::Backend(
                "lookup returned non-lookup response".into(),
            )),
        }
    }

    fn list(ctx: Ctx) -> Result<Vec<Entry>, RouteError> {
        match list(&ctx.path) {
            DispatchResponse::List(entries) => Ok(entries.into_iter().map(route_entry).collect()),
            DispatchResponse::Error { code, message } => Err(route_error(code, message)),
            _ => Err(RouteError::Backend(
                "list returned non-list response".into(),
            )),
        }
    }

    fn read(ctx: Ctx) -> Result<Vec<u8>, RouteError> {
        match read(&ctx.path) {
            DispatchResponse::Read(bytes) => Ok(bytes),
            DispatchResponse::Error { code, message } => Err(route_error(code, message)),
            _ => Err(RouteError::Backend(
                "read returned non-read response".into(),
            )),
        }
    }

    fn write(ctx: Ctx, body: Vec<u8>) -> Result<(), RouteError> {
        match write(&ctx.path, &body) {
            DispatchResponse::Write => Ok(()),
            DispatchResponse::Error { code, message } => Err(route_error(code, message)),
            _ => Err(RouteError::Backend(
                "write returned non-write response".into(),
            )),
        }
    }
}

#[cfg(not(test))]
export!(Route);

fn component_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let bytes =
        bloom_petal_sdk::random_bytes(buf.len()).map_err(|_| getrandom::Error::UNSUPPORTED)?;
    buf.copy_from_slice(&bytes);
    Ok(())
}

getrandom::register_custom_getrandom!(component_getrandom);

fn metadata_path(path: &str) -> String {
    match path {
        "$index" | "$list" => String::new(),
        _ => path
            .strip_suffix("/$index")
            .or_else(|| path.strip_suffix("/$list"))
            .unwrap_or(path)
            .to_string(),
    }
}

fn route_cache_ttl_ms(path: &str) -> Option<u64> {
    let segs = split(path);
    match segs.first().copied() {
        Some("onboard") => None,
        Some("account") => Some(5_000),
        Some("markets")
            if matches!(
                segs.get(2).copied(),
                Some("book.json") | Some("prices.json")
            ) =>
        {
            Some(2_000)
        }
        Some("positions") => Some(10_000),
        _ => Some(30_000),
    }
}

fn route_required_caps(path: &str, _writable: bool) -> Vec<String> {
    let mut caps = vec![
        "bloom:http".to_string(),
        "bloom:store".to_string(),
        "bloom:vfs.read".to_string(),
    ];
    if path.starts_with("onboard/")
        || path.starts_with("trade/")
        || path.starts_with("redeem/")
        || path.starts_with("revoke-approvals/")
        || path.starts_with("withdraw/")
    {
        caps.push("bloom:sign".to_string());
        caps.push("bloom:vfs.write".to_string());
    }
    if path.starts_with("fund/") {
        caps.push("bloom:tx.outbox".to_string());
    }
    if path.starts_with("onboard/")
        || path.starts_with("account/")
        || path.starts_with("fund/")
        || path.starts_with("trade/")
        || path.starts_with("redeem/")
        || path.starts_with("revoke-approvals/")
        || path.starts_with("withdraw/")
    {
        caps.push("bloom:chain".to_string());
    }
    caps
}

fn route_entry(entry: DispatchEntry) -> Entry {
    Entry {
        name: entry.name,
        kind: match entry.kind {
            DispatchEntryKind::Dir => EntryKind::Dir,
            DispatchEntryKind::File | DispatchEntryKind::WritableFile => EntryKind::File,
        },
        mode: entry.mode,
        size: Some(entry.size),
        link_target: entry.link_target,
    }
}

fn route_error(code: i32, message: String) -> RouteError {
    match code {
        -1 => RouteError::NotFound(message),
        -2 => RouteError::Denied(message),
        -3 => RouteError::Invalid(message),
        -4 => RouteError::Backend(message),
        _ => RouteError::Unsupported(message),
    }
}

mod bloom_petal_sdk {
    use crate::bloom::chain::read as chain;
    #[allow(unused_imports)]
    use crate::bloom::env::runtime as env;
    use crate::bloom::http::fetch as http;
    #[cfg(not(test))]
    use crate::bloom::sign::signing as sign;
    use crate::bloom::store::kv as store;
    use crate::bloom::tx::outbox as tx;
    use crate::bloom::vfs::readwrite as vfs;

    const STATE_NS: &str = "state";
    const SECRET_NS: &str = "secrets";

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum DispatchEntryKind {
        Dir,
        File,
        WritableFile,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DispatchEntry {
        pub name: String,
        pub kind: DispatchEntryKind,
        pub size: u64,
        pub mode: u32,
        pub ttl_hint_ms: Option<u64>,
        pub link_target: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum DispatchOp {
        Lookup,
        List,
        Read,
        Write,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct DispatchRequest {
        pub op: DispatchOp,
        pub path: String,
        pub body: Vec<u8>,
        pub ctx: Vec<(String, String)>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum DispatchResponse {
        Lookup(DispatchEntry),
        List(Vec<DispatchEntry>),
        Read(Vec<u8>),
        Write,
        Error { code: i32, message: String },
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HttpRequest {
        pub method: String,
        pub url: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HttpResponse {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SignRequest {
        pub wallet: String,
        pub hash32: [u8; 32],
        pub purpose: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum SignHashOutcome {
        Signature(Vec<u8>),
        ApprovalRequired {
            action_id: String,
            ceremony_url: String,
            expires_ms: u64,
        },
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct EvmTransaction {
        pub wallet: String,
        pub chain: String,
        pub to: String,
        pub value_wei: String,
        pub data_hex: String,
        pub nonce: Option<u64>,
        pub max_fee_per_gas: Option<String>,
        pub max_priority_fee_per_gas: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OutboxApproval {
        pub action_id: String,
        pub ceremony_url: String,
        pub expires_ms: u64,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct StagedTransaction {
        pub outbox_id: String,
        pub plan_md: String,
        pub approval: Option<OutboxApproval>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OutboxInspection {
        pub outbox_id: String,
        pub state: String,
        pub tx_hash: Option<String>,
        pub receipt_json: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum HostStatus {
        NotFound,
        Denied,
        Invalid,
        Backend,
        BufferTooSmall { needed: usize },
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum SdkError {
        Host(HostStatus),
        Message(String),
    }

    impl SdkError {
        pub fn message(&self) -> String {
            match self {
                SdkError::Host(HostStatus::NotFound) => "not found".into(),
                SdkError::Host(HostStatus::Denied) => "denied".into(),
                SdkError::Host(HostStatus::Invalid) => "invalid".into(),
                SdkError::Host(HostStatus::Backend) => "backend error".into(),
                SdkError::Host(HostStatus::BufferTooSmall { needed }) => {
                    format!("buffer too small: needs {needed} bytes")
                }
                SdkError::Message(message) => message.clone(),
            }
        }
    }

    pub fn http_fetch(req: &HttpRequest, max_bytes: usize) -> Result<HttpResponse, SdkError> {
        let resp = http::fetch(&http::Request {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            body: req.body.clone(),
        })
        .map_err(host_err)?;
        if resp.body.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: resp.body.len(),
            }));
        }
        Ok(HttpResponse {
            status: resp.status,
            headers: resp.headers,
            body: resp.body,
        })
    }

    #[cfg(not(test))]
    pub fn sign_hash(req: &SignRequest) -> Result<SignHashOutcome, SdkError> {
        match sign::sign_hash(&req.wallet, &req.hash32, &req.purpose).map_err(host_err)? {
            sign::SignResult::Signature(signature) => Ok(SignHashOutcome::Signature(signature)),
            sign::SignResult::ApprovalRequired(approval) => Ok(SignHashOutcome::ApprovalRequired {
                action_id: approval.action_id,
                ceremony_url: approval.ceremony_url,
                expires_ms: approval.expires_ms,
            }),
        }
    }

    #[cfg(test)]
    pub fn sign_hash(_req: &SignRequest) -> Result<SignHashOutcome, SdkError> {
        TEST_SIGN_OUTCOMES.with(|outcomes| {
            outcomes
                .borrow_mut()
                .pop_front()
                .expect("mock signing outcome")
        })
    }

    #[cfg(not(test))]
    #[allow(dead_code)]
    pub fn tx_stage(req: &EvmTransaction) -> Result<StagedTransaction, SdkError> {
        let staged = tx::stage(&tx::EvmTransaction {
            wallet: req.wallet.clone(),
            chain: req.chain.clone(),
            to: req.to.clone(),
            value_wei: req.value_wei.clone(),
            data_hex: req.data_hex.clone(),
            nonce: req.nonce,
            max_fee_per_gas: req.max_fee_per_gas.clone(),
            max_priority_fee_per_gas: req.max_priority_fee_per_gas.clone(),
        })
        .map_err(host_err)?;
        Ok(staged_transaction(staged))
    }

    #[cfg(test)]
    pub fn tx_stage(req: &EvmTransaction) -> Result<StagedTransaction, SdkError> {
        TEST_TX_STAGE_CALLS.with(|calls| calls.borrow_mut().push(req.clone()));
        TEST_TX_STAGE_OUTCOMES.with(|outcomes| {
            outcomes
                .borrow_mut()
                .pop_front()
                .expect("mock tx stage outcome")
        })
    }

    #[cfg(not(test))]
    #[allow(dead_code)]
    pub fn tx_confirm(
        wallet: &str,
        chain: &str,
        outbox_id: &str,
        acknowledge_warnings: bool,
    ) -> Result<StagedTransaction, SdkError> {
        tx::confirm(wallet, chain, outbox_id, acknowledge_warnings)
            .map(staged_transaction)
            .map_err(host_err)
    }

    #[cfg(test)]
    pub fn tx_confirm(
        wallet: &str,
        chain: &str,
        outbox_id: &str,
        acknowledge_warnings: bool,
    ) -> Result<StagedTransaction, SdkError> {
        TEST_TX_CONFIRM_CALLS.with(|calls| {
            calls.borrow_mut().push((
                wallet.into(),
                chain.into(),
                outbox_id.into(),
                acknowledge_warnings,
            ));
        });
        TEST_TX_CONFIRM_OUTCOMES.with(|outcomes| {
            outcomes
                .borrow_mut()
                .pop_front()
                .expect("mock tx confirm outcome")
        })
    }

    #[cfg(not(test))]
    pub fn tx_inspect(
        wallet: &str,
        chain: &str,
        outbox_id: &str,
    ) -> Result<OutboxInspection, SdkError> {
        tx::inspect(wallet, chain, outbox_id)
            .map(|inspection| OutboxInspection {
                outbox_id: inspection.outbox_id,
                state: inspection.state,
                tx_hash: inspection.tx_hash,
                receipt_json: inspection.receipt_json,
            })
            .map_err(host_err)
    }

    #[cfg(test)]
    pub fn tx_inspect(
        wallet: &str,
        chain: &str,
        outbox_id: &str,
    ) -> Result<OutboxInspection, SdkError> {
        TEST_TX_INSPECT_CALLS.with(|calls| {
            calls
                .borrow_mut()
                .push((wallet.into(), chain.into(), outbox_id.into()));
        });
        TEST_TX_INSPECT_OUTCOMES.with(|outcomes| {
            outcomes
                .borrow_mut()
                .pop_front()
                .expect("mock tx inspect outcome")
        })
    }

    pub fn chain_read(
        chain_name: &str,
        method: &str,
        params_json: &str,
    ) -> Result<String, SdkError> {
        chain::call(&chain::Request {
            chain: chain_name.into(),
            method: method.into(),
            params_json: params_json.into(),
        })
        .map(|response| response.result_json)
        .map_err(host_err)
    }

    #[allow(dead_code)]
    fn staged_transaction(staged: tx::StagedTransaction) -> StagedTransaction {
        StagedTransaction {
            outbox_id: staged.outbox_id,
            plan_md: staged.plan_md,
            approval: staged.approval.map(|approval| OutboxApproval {
                action_id: approval.action_id,
                ceremony_url: approval.ceremony_url,
                expires_ms: approval.expires_ms,
            }),
        }
    }

    #[cfg(not(test))]
    pub fn store_get(key: &str, max_bytes: usize) -> Result<Vec<u8>, SdkError> {
        let namespace = namespace_for_key(key, false);
        let Some(bytes) = store::get(namespace, key).map_err(host_err)? else {
            return Err(SdkError::Host(HostStatus::NotFound));
        };
        if bytes.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: bytes.len(),
            }));
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub fn store_get(key: &str, max_bytes: usize) -> Result<Vec<u8>, SdkError> {
        TEST_STORE.with(|store| {
            let bytes = store
                .borrow()
                .get(key)
                .cloned()
                .ok_or(SdkError::Host(HostStatus::NotFound))?;
            if bytes.len() > max_bytes {
                return Err(SdkError::Host(HostStatus::BufferTooSmall {
                    needed: bytes.len(),
                }));
            }
            Ok(bytes)
        })
    }

    #[cfg(not(test))]
    pub fn store_put(key: &str, value: &[u8], secret: bool) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, secret);
        store::put(namespace, key, value, namespace == SECRET_NS).map_err(host_err)
    }

    #[cfg(test)]
    pub fn store_put(key: &str, value: &[u8], _secret: bool) -> Result<(), SdkError> {
        TEST_STORE.with(|store| {
            store.borrow_mut().insert(key.into(), value.to_vec());
        });
        Ok(())
    }

    #[cfg(test)]
    thread_local! {
        static TEST_SIGN_OUTCOMES: std::cell::RefCell<std::collections::VecDeque<Result<SignHashOutcome, SdkError>>> =
            const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
        static TEST_STORE: std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>> =
            const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
        static TEST_TX_STAGE_OUTCOMES: std::cell::RefCell<std::collections::VecDeque<Result<StagedTransaction, SdkError>>> =
            const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
        static TEST_TX_CONFIRM_OUTCOMES: std::cell::RefCell<std::collections::VecDeque<Result<StagedTransaction, SdkError>>> =
            const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
        static TEST_TX_INSPECT_OUTCOMES: std::cell::RefCell<std::collections::VecDeque<Result<OutboxInspection, SdkError>>> =
            const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
        static TEST_TX_STAGE_CALLS: std::cell::RefCell<Vec<EvmTransaction>> = const { std::cell::RefCell::new(Vec::new()) };
        static TEST_TX_CONFIRM_CALLS: std::cell::RefCell<Vec<(String, String, String, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
        static TEST_TX_INSPECT_CALLS: std::cell::RefCell<Vec<(String, String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    #[cfg(test)]
    pub fn test_host_reset(sign_outcomes: Vec<Result<SignHashOutcome, SdkError>>) {
        TEST_SIGN_OUTCOMES.with(|outcomes| {
            *outcomes.borrow_mut() = sign_outcomes.into();
        });
        TEST_STORE.with(|store| store.borrow_mut().clear());
        TEST_TX_STAGE_OUTCOMES.with(|outcomes| outcomes.borrow_mut().clear());
        TEST_TX_CONFIRM_OUTCOMES.with(|outcomes| outcomes.borrow_mut().clear());
        TEST_TX_INSPECT_OUTCOMES.with(|outcomes| outcomes.borrow_mut().clear());
        TEST_TX_STAGE_CALLS.with(|calls| calls.borrow_mut().clear());
        TEST_TX_CONFIRM_CALLS.with(|calls| calls.borrow_mut().clear());
        TEST_TX_INSPECT_CALLS.with(|calls| calls.borrow_mut().clear());
    }

    #[cfg(test)]
    pub fn test_host_set_tx_outcomes(
        stage: Vec<Result<StagedTransaction, SdkError>>,
        confirm: Vec<Result<StagedTransaction, SdkError>>,
        inspect: Vec<Result<OutboxInspection, SdkError>>,
    ) {
        TEST_TX_STAGE_OUTCOMES.with(|outcomes| *outcomes.borrow_mut() = stage.into());
        TEST_TX_CONFIRM_OUTCOMES.with(|outcomes| *outcomes.borrow_mut() = confirm.into());
        TEST_TX_INSPECT_OUTCOMES.with(|outcomes| *outcomes.borrow_mut() = inspect.into());
    }

    #[cfg(test)]
    pub fn test_host_tx_call_counts() -> (usize, usize, usize) {
        (
            TEST_TX_STAGE_CALLS.with(|calls| calls.borrow().len()),
            TEST_TX_CONFIRM_CALLS.with(|calls| calls.borrow().len()),
            TEST_TX_INSPECT_CALLS.with(|calls| calls.borrow().len()),
        )
    }

    pub fn store_put_new(key: &str, value: &[u8], secret: bool) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, secret);
        store::put_new(namespace, key, value, namespace == SECRET_NS).map_err(host_err)
    }

    pub fn store_del(key: &str) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, false);
        store::delete(namespace, key).map_err(host_err)
    }

    pub fn store_del_if_value(key: &str, expected: &[u8]) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, false);
        store::delete_if_value(namespace, key, expected).map_err(host_err)
    }

    pub fn store_list(prefix: &str, max_bytes: usize) -> Result<Vec<String>, SdkError> {
        let namespace = namespace_for_key(prefix, false);
        let keys = store::list(namespace, prefix).map_err(host_err)?;
        let size = keys.iter().map(|key| key.len()).sum::<usize>();
        if size > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall { needed: size }));
        }
        Ok(keys)
    }

    pub fn vfs_read(path: &str, max_bytes: usize) -> Result<Vec<u8>, SdkError> {
        let bytes = vfs::read(path).map_err(host_err)?;
        if bytes.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: bytes.len(),
            }));
        }
        Ok(bytes)
    }

    pub fn vfs_write(path: &str, body: &[u8]) -> Result<(), SdkError> {
        vfs::write(path, body).map_err(host_err)
    }

    pub fn vfs_list(path: &str, max_bytes: usize) -> Result<Vec<String>, SdkError> {
        let _ = vfs::lookup(path).map_err(host_err)?;
        let entries = vfs::list(path).map_err(host_err)?;
        let size = entries.iter().map(|entry| entry.name.len()).sum::<usize>();
        if size > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall { needed: size }));
        }
        Ok(entries.into_iter().map(|entry| entry.name).collect())
    }

    #[cfg(not(test))]
    pub fn now_ms() -> u64 {
        env::now_ms().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn now_ms() -> u64 {
        100
    }

    pub fn random_bytes(len: usize) -> Result<Vec<u8>, SdkError> {
        let len = u32::try_from(len).map_err(|_| SdkError::Host(HostStatus::Invalid))?;
        env::random_bytes(len).map_err(host_err)
    }

    fn namespace_for_key(key: &str, secret: bool) -> &'static str {
        if secret || key.starts_with("creds/") {
            SECRET_NS
        } else {
            STATE_NS
        }
    }

    fn host_err(message: String) -> SdkError {
        let lower = message.to_ascii_lowercase();
        // `signing@0.1` has no structured approval result. Preserve the
        // daemon's approval-required payload so the route can project a
        // redacted retry artifact for the caller.
        if lower.contains("sealed approval required") {
            SdkError::Message(message)
        } else if lower.contains("not found") {
            SdkError::Host(HostStatus::NotFound)
        } else if lower.contains("denied") || lower.contains("permission") {
            SdkError::Host(HostStatus::Denied)
        } else if lower.contains("invalid") {
            SdkError::Host(HostStatus::Invalid)
        } else {
            SdkError::Message(message)
        }
    }
}

use std::collections::BTreeSet;

use alloy::primitives::{Address, B256, Signature, U256};
use bloom_petal_sdk::{
    DispatchEntry, DispatchEntryKind, DispatchOp, DispatchRequest, DispatchResponse,
    EvmTransaction, HostStatus, HttpRequest, SdkError, SignHashOutcome, SignRequest,
};
use bloom_polymarket::eip712::{
    Batch, CTF, CTF_COLLATERAL_ADAPTER, CTF_EXCHANGE_V2, Call, FACTORY,
    NEG_RISK_CTF_COLLATERAL_ADAPTER, NEG_RISK_EXCHANGE_V2, PUSD, batch_signing_hash,
    clob_auth_signing_hash, derive_deposit_wallet_address,
};
use bloom_polymarket::order::{
    LimitQuote, Order, OrderBody, OrderParams, OrderType, SIG_TYPE_POLY_1271, build_order,
    format_micro, parse_micro, poly1271_digest, wrap_poly1271_signature,
};
use bloom_polymarket::signer::{
    POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP, l2_headers,
};
use bloom_polymarket::trade as shared_trade;
use bloom_polymarket::types::{Market, Side};
use bloom_polymarket::wallet::{
    V2_APPROVAL_LABELS, redeem_positions_call, transfer_amount_call, v2_approval_calls,
    v2_revoke_calls,
};
use bloom_polymarket::{
    BuilderCredentials, Credentials, OrderBook, POLYGON, Position, Trade, validate_wallet_name,
};
use serde::{Deserialize, Serialize};
use url::Url;

const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;
const MAX_STORE_BYTES: usize = 1024 * 1024;
const MAX_LIST_BYTES: usize = 256 * 1024;
const MAX_POLICY_BYTES: usize = 256 * 1024;
const MAX_CHAIN_METHOD_BYTES: usize = 256 * 1024;
const MAX_CHAIN_READ_BYTES: usize = 256 * 1024;
const MARKETS_LIST_LIMIT: u32 = 20;
const TRADE_LOCK_STALE_MS: u128 = 5 * 60 * 1000;

const GAMMA: &str = "https://gamma-api.polymarket.com";
const DATA: &str = "https://data-api.polymarket.com";
const CLOB: &str = "https://clob.polymarket.com";
const RELAYER: &str = "https://relayer-v2.polymarket.com";
const CLOB_AUTH_NONCE: u32 = 0;
const ONBOARD_IN_FLIGHT_TIMEOUT_SECS: u64 = 180;
const BATCH_DEADLINE_SECS: u64 = 3600;

const ROOT_DIRS: [&str; 14] = [
    "markets",
    "search",
    "positions",
    "onboard",
    "account",
    "builder-keys",
    "trade",
    "fund",
    "redeem",
    "revoke-approvals",
    "withdraw",
    "obligations",
    "settings",
    "meta",
];
const META_FILES: [&str; 2] = ["parity.json", "route-contract.json"];
const MARKET_FILES: [&str; 3] = ["market.json", "book.json", "prices.json"];
const POSITION_FILES: [&str; 3] = ["positions.json", "trades.json", "activity.json"];
const ONBOARD_FILES: [&str; 5] = [
    "status.json",
    "plan.md",
    "approvals.json",
    "review_intent.json",
    "approval.json",
];
const ACCOUNT_FILES: [&str; 5] = [
    "portfolio.json",
    "orders.json",
    "status.json",
    "buying_power.json",
    "funding_options.json",
];
const BUILDER_KEY_FILES: [&str; 1] = ["keys.json"];
const BUILDER_KEY_WRITABLE_FILES: [&str; 1] = ["revoke"];
const SETTINGS_WRITABLE_FILES: [&str; 1] = ["enso-api-key"];
const FUND_FILES: [&str; 5] = [
    "plan.md",
    "request.json",
    "status.json",
    "review_intent.json",
    "approval.json",
];
const FUND_WRITABLE_FILES: [&str; 1] = ["confirm"];
const RELAYER_ACTION_FILES: [&str; 4] = [
    "plan.md",
    "review_intent.json",
    "approval.json",
    "receipt.json",
];
const RELAYER_ACTION_WRITABLE_FILES: [&str; 1] = ["confirm"];
const DRAFT_FILES: [&str; 7] = [
    "plan.md",
    "order.json",
    "policy_check.json",
    "quote.json",
    "review_intent.json",
    "post_attempt.json",
    "approval.json",
];
const DRAFT_WRITABLE_FILES: [&str; 2] = ["revalidate", "post"];
const RECEIPT_FILES: [&str; 1] = ["receipt.json"];
const RECEIPT_WRITABLE_FILES: [&str; 1] = ["cancel"];

const BEGIN_HINT: &str =
    "write anything here to mint or derive CLOB credentials with the daemon keystore\n";
const TRADE_NEW_HINT: &str = r#"write JSON to create a reviewable draft, e.g.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","amount":"1","max_price":"0.01"}
"#;
const FUND_NEW_HINT: &str = r#"write JSON to create a reviewable pUSD funding request, e.g.
{"target_pusd":"10","max_spend":"100","from_token":"native","slippage_bps":50}
"#;
const TRADE_REVALIDATE_HINT: &str = r#"write {"revalidate":true} to revalidate this draft and stage the final review artifact. Revalidated drafts can then be posted by writing {"post":true} to post; resting GTC orders can be cancelled from their receipt.
"#;
const TRADE_POST_HINT: &str = r#"write {"post":true} to sign and post a revalidated draft, then write a private receipt. This performs a value-moving CLOB POST /order.
"#;
const TRADE_CANCEL_HINT: &str = r#"write {"cancel":true} to cancel the posted CLOB order recorded by this receipt. Cancelling uses CLOB DELETE /order and updates the private receipt/draft status.
"#;
const ORDER_CANCEL_HINT: &str = r#"write "confirm" or {"cancel":true} to cancel this CLOB order. The order must still be discoverable from account/<wallet>/orders.json.
"#;
const BUILDER_KEY_REVOKE_HINT: &str = r#"write "confirm" to revoke the account builder key, or JSON with an explicit key id:
{"confirm":true,"key":"<builder-key-id>"}
"#;

pub fn handle(req: DispatchRequest) -> DispatchResponse {
    let relative = match validate_relative_path(&req.path) {
        Ok(path) => path,
        Err(message) => return error(-3, message),
    };
    match req.op {
        DispatchOp::Lookup => lookup(relative),
        DispatchOp::List => list(relative),
        DispatchOp::Read => read(relative),
        DispatchOp::Write => write(relative, &req.body),
    }
}

fn lookup(relative: &str) -> DispatchResponse {
    match path_kind(relative) {
        Some(kind) => DispatchResponse::Lookup(entry(entry_name(relative), kind)),
        None => error(-1, "not found"),
    }
}

fn list(relative: &str) -> DispatchResponse {
    if path_kind(relative) != Some(DispatchEntryKind::Dir) {
        return error(-3, "not a directory");
    }
    let segs = split(relative);
    let names = match (segs.first().copied(), segs.len()) {
        (None, 0) => ROOT_DIRS.iter().map(|s| (*s).to_string()).collect(),
        (Some("markets"), 1) => match list_market_slugs() {
            Ok(slugs) => slugs,
            Err(resp) => return resp,
        },
        (Some("markets"), 2) => strings(&MARKET_FILES),
        (Some("meta"), 1) => strings(&META_FILES),
        (Some("positions"), 1) => vfs_wallets_or_store(""),
        (Some("positions"), 2) => strings(&POSITION_FILES),
        (Some("onboard"), 1) => vfs_wallets_or_store("onboard/"),
        (Some("onboard"), 2) => {
            let mut out = vec!["begin".to_string()];
            out.extend(strings(&ONBOARD_FILES));
            out
        }
        (Some("account"), 1) => vfs_wallets_or_store("creds/"),
        (Some("account"), 2) => strings(&ACCOUNT_FILES),
        (Some("builder-keys"), 1) => vfs_wallets_or_store("creds/"),
        (Some("builder-keys"), 2) => {
            let mut out = strings(&BUILDER_KEY_FILES);
            out.extend(strings(&BUILDER_KEY_WRITABLE_FILES));
            out
        }
        (Some("settings"), 1) => strings(&SETTINGS_WRITABLE_FILES),
        (Some("fund"), 1) => vfs_wallets_or_store("fund/"),
        (Some("fund"), 2) => {
            let mut out = vec!["new".to_string()];
            out.extend(store_ids(&format!("fund/{}/requests/", segs[1]), ".json"));
            out
        }
        (Some("fund"), 3) if segs[2] != "new" => {
            let mut out = strings(&FUND_FILES);
            out.extend(strings(&FUND_WRITABLE_FILES));
            out
        }
        (Some("redeem"), 1) => vfs_wallets_or_store("onboard/"),
        (Some("redeem"), 2) => Vec::new(),
        (Some("redeem"), 3) => {
            let mut out = strings(&RELAYER_ACTION_FILES);
            out.extend(strings(&RELAYER_ACTION_WRITABLE_FILES));
            out
        }
        (Some("revoke-approvals"), 1) => vfs_wallets_or_store("onboard/"),
        (Some("revoke-approvals"), 2) => vec!["request".into()],
        (Some("revoke-approvals"), 3) if segs[2] == "request" => {
            let mut out = strings(&RELAYER_ACTION_FILES);
            out.extend(strings(&RELAYER_ACTION_WRITABLE_FILES));
            out
        }
        (Some("withdraw"), 1) => vfs_wallets_or_store("onboard/"),
        (Some("withdraw"), 2) => vec!["pusd".into()],
        (Some("withdraw"), 3) if segs[2] == "pusd" => {
            let mut out = strings(&RELAYER_ACTION_FILES);
            out.extend(strings(&RELAYER_ACTION_WRITABLE_FILES));
            out
        }
        (Some("trade"), 1) => vfs_wallets_or_store("trade/"),
        (Some("trade"), 2) => vec![
            "new".into(),
            "drafts".into(),
            "orders".into(),
            "receipts".into(),
        ],
        (Some("trade"), 3) if segs[2] == "drafts" => {
            store_ids(&format!("trade/{}/drafts/", segs[1]), "/order.json")
        }
        (Some("trade"), 3) if segs[2] == "receipts" => {
            store_ids(&format!("trade/{}/receipts/", segs[1]), "/receipt.json")
        }
        (Some("trade"), 3) if segs[2] == "orders" => {
            match list_discoverable_clob_order_ids(segs[1]) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            }
        }
        (Some("trade"), 4) if segs[2] == "drafts" => {
            let mut out = strings(&DRAFT_FILES);
            out.extend(strings(&DRAFT_WRITABLE_FILES));
            out
        }
        (Some("trade"), 4) if segs[2] == "receipts" => {
            let mut out = strings(&RECEIPT_FILES);
            out.extend(strings(&RECEIPT_WRITABLE_FILES));
            out
        }
        (Some("trade"), 4) if segs[2] == "orders" => vec!["cancel".into()],
        (Some("obligations"), 1) => vfs_wallets_or_store("onboard/")
            .into_iter()
            .map(|wallet| format!("{wallet}.json"))
            .collect(),
        _ => Vec::new(),
    };
    DispatchResponse::List(
        names
            .into_iter()
            .filter(|name| is_safe_segment(name))
            .filter_map(|name| {
                let child = child_relative(relative, &name);
                path_kind(&child).map(|kind| entry(&name, kind))
            })
            .collect(),
    )
}

fn read(relative: &str) -> DispatchResponse {
    if !matches!(
        path_kind(relative),
        Some(DispatchEntryKind::File | DispatchEntryKind::WritableFile)
    ) {
        return error(-3, "not a file");
    }
    let segs = split(relative);
    match (segs.first().copied(), segs.len()) {
        (Some("markets"), 3) => read_market(segs[1], segs[2]),
        (Some("meta"), 2) => read_meta(segs[1]),
        (Some("search"), 2) => read_search(segs[1]),
        (Some("positions"), 3) => read_positions(segs[1], segs[2]),
        (Some("onboard"), 3) => read_onboard(segs[1], segs[2]),
        (Some("account"), 3) => read_account(segs[1], segs[2]),
        (Some("builder-keys"), 3) => read_builder_keys(segs[1], segs[2]),
        (Some("settings"), 2) if segs[1] == "enso-api-key" => DispatchResponse::Read(
            b"write an Enso API key here; the value is stored privately and is never readable\n"
                .to_vec(),
        ),
        (Some("obligations"), 2) if segs[1].ends_with(".json") => {
            read_obligations(segs[1].trim_end_matches(".json"))
        }
        (Some("fund"), 3) if segs[2] == "new" => DispatchResponse::Read(FUND_NEW_HINT.into()),
        (Some("fund"), 4) if segs[3] == "confirm" => DispatchResponse::Read(
            b"write confirm to stage and approve the exact funding transaction\n".to_vec(),
        ),
        (Some("fund"), 4) => read_fund(segs[1], segs[2], segs[3]),
        (Some("redeem"), 4) if segs[3] == "plan.md" => read_redeem_plan(segs[1], segs[2]),
        (Some("redeem"), 4) if segs[3] == "approval.json" => read_store(&format!(
            "actions/{}/redeem/{}/approval.json",
            segs[1], segs[2]
        )),
        (Some("redeem"), 4) if segs[3] == "review_intent.json" => read_store(&format!(
            "actions/{}/redeem/{}/review_intent.json",
            segs[1], segs[2]
        )),
        (Some("redeem"), 4) if segs[3] == "receipt.json" => read_store(&format!(
            "actions/{}/redeem/{}/receipt.json",
            segs[1], segs[2]
        )),
        (Some("redeem"), 4) if segs[3] == "confirm" => DispatchResponse::Read(
            b"write confirm to sign and submit the exact persisted redemption batch\n".to_vec(),
        ),
        (Some("revoke-approvals"), 4) if segs[2] == "request" && segs[3] == "plan.md" => {
            read_revoke_approvals_plan(segs[1])
        }
        (Some("revoke-approvals"), 4) if segs[2] == "request" && segs[3] == "approval.json" => {
            read_store(&format!(
                "actions/{}/revoke-approvals/approval.json",
                segs[1]
            ))
        }
        (Some("revoke-approvals"), 4)
            if segs[2] == "request" && segs[3] == "review_intent.json" =>
        {
            read_store(&format!(
                "actions/{}/revoke-approvals/review_intent.json",
                segs[1]
            ))
        }
        (Some("revoke-approvals"), 4) if segs[2] == "request" && segs[3] == "receipt.json" => {
            read_store(&format!(
                "actions/{}/revoke-approvals/receipt.json",
                segs[1]
            ))
        }
        (Some("revoke-approvals"), 4) if segs[2] == "request" && segs[3] == "confirm" => {
            DispatchResponse::Read(
                b"write confirm to sign and submit the exact persisted approval-revocation batch\n"
                    .to_vec(),
            )
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "plan.md" => {
            read_withdraw_pusd_plan(segs[1])
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "approval.json" => {
            read_store(&format!("actions/{}/withdraw-pusd/approval.json", segs[1]))
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "review_intent.json" => {
            read_store(&format!(
                "actions/{}/withdraw-pusd/review_intent.json",
                segs[1]
            ))
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "receipt.json" => {
            read_store(&format!("actions/{}/withdraw-pusd/receipt.json", segs[1]))
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "confirm" => {
            DispatchResponse::Read(
                b"write confirm to sign and submit the exact persisted pUSD withdrawal batch\n"
                    .to_vec(),
            )
        }
        (Some("trade"), 3) if segs[2] == "new" => DispatchResponse::Read(TRADE_NEW_HINT.into()),
        (Some("trade"), 5) if segs[2] == "orders" && segs[4] == "cancel" => {
            DispatchResponse::Read(ORDER_CANCEL_HINT.into())
        }
        (Some("trade"), 5) => read_trade(segs[1], segs[2], segs[3], segs[4]),
        _ => error(-3, "not a file"),
    }
}

fn write(relative: &str, body: &[u8]) -> DispatchResponse {
    if path_kind(relative) != Some(DispatchEntryKind::WritableFile) {
        return error(-2, "path is not writable");
    }
    let segs = split(relative);
    match (segs.first().copied(), segs.len()) {
        (Some("onboard"), 3) if segs[2] == "begin" => write_onboard_begin(segs[1]),
        (Some("trade"), 3) if segs[2] == "new" => write_trade_new(segs[1], body),
        (Some("trade"), 5) if segs[2] == "drafts" && segs[4] == "revalidate" => {
            write_trade_revalidate(segs[1], segs[3], body)
        }
        (Some("trade"), 5) if segs[2] == "drafts" && segs[4] == "post" => {
            write_trade_post(segs[1], segs[3], body)
        }
        (Some("trade"), 5) if segs[2] == "receipts" && segs[4] == "cancel" => {
            write_trade_cancel(segs[1], segs[3], body)
        }
        (Some("trade"), 5) if segs[2] == "orders" && segs[4] == "cancel" => {
            write_discovered_trade_cancel(segs[1], segs[3], body)
        }
        (Some("builder-keys"), 3) if segs[2] == "revoke" => write_builder_key_revoke(segs[1], body),
        (Some("settings"), 2) if segs[1] == "enso-api-key" => write_enso_api_key(body),
        (Some("fund"), 3) if segs[2] == "new" => write_fund_new(segs[1], body),
        (Some("fund"), 4) if segs[3] == "confirm" => write_fund_confirm(segs[1], segs[2], body),
        (Some("redeem"), 4) if segs[3] == "confirm" => write_redeem_confirm(segs[1], segs[2], body),
        (Some("revoke-approvals"), 4) if segs[2] == "request" && segs[3] == "confirm" => {
            write_revoke_approvals_confirm(segs[1], body)
        }
        (Some("withdraw"), 4) if segs[2] == "pusd" && segs[3] == "confirm" => {
            write_withdraw_pusd_confirm(segs[1], body)
        }
        _ => error(-2, "path is not writable"),
    }
}

fn read_meta(file: &str) -> DispatchResponse {
    match file {
        "route-contract.json" => read_json_value(&serde_json::json!({
            "schema": "bloom.polymarket.petal-route-contract.v1",
            "legacy_root_forbidden": "polymarket/",
            "routes": {
                "market": "markets/<slug>/market.json",
                "market_book": "markets/<slug>/book.json",
                "market_prices": "markets/<slug>/prices.json",
                "search": "search/<query>",
                "positions": "positions/<wallet>/positions.json",
                "trades": "positions/<wallet>/trades.json",
                "activity": "positions/<wallet>/activity.json",
                "onboard_plan": "onboard/<wallet>/plan.md",
                "onboard_status": "onboard/<wallet>/status.json",
                "onboard_approvals": "onboard/<wallet>/approvals.json",
                "onboard_review": "onboard/<wallet>/review_intent.json",
                "account_status": "account/<wallet>/status.json",
                "account_portfolio": "account/<wallet>/portfolio.json",
                "account_orders": "account/<wallet>/orders.json",
                "buying_power": "account/<wallet>/buying_power.json",
                "funding_options": "account/<wallet>/funding_options.json",
                "builder_keys": "builder-keys/<wallet>/keys.json",
                "builder_key_revoke": "builder-keys/<wallet>/revoke",
                "enso_settings": "settings/enso-api-key",
                "trade_new": "trade/<wallet>/new",
                "trade_plan": "trade/<wallet>/drafts/<id>/plan.md",
                "trade_order": "trade/<wallet>/drafts/<id>/order.json",
                "trade_quote": "trade/<wallet>/drafts/<id>/quote.json",
                "trade_policy": "trade/<wallet>/drafts/<id>/policy_check.json",
                "trade_revalidate": "trade/<wallet>/drafts/<id>/revalidate",
                "trade_review": "trade/<wallet>/drafts/<id>/review_intent.json",
                "trade_post_attempt": "trade/<wallet>/drafts/<id>/post_attempt.json",
                "trade_post": "trade/<wallet>/drafts/<id>/post",
                "trade_receipt": "trade/<wallet>/receipts/<id>/receipt.json",
                "trade_receipt_cancel": "trade/<wallet>/receipts/<id>/cancel",
                "fund_prepare_execute": "fund/<wallet>/<id>/confirm",
                "fund_new": "fund/<wallet>/new",
                "fund_plan": "fund/<wallet>/<id>/plan.md",
                "fund_request": "fund/<wallet>/<id>/request.json",
                "fund_status": "fund/<wallet>/<id>/status.json",
                "fund_review": "fund/<wallet>/<id>/review_intent.json",
                "fund_approval": "fund/<wallet>/<id>/approval.json",
                "arbitrary_order_cancel": "trade/<wallet>/orders/<clob-order-id>/cancel",
                "order_approval": "trade/<wallet>/drafts/<id>/approval.json",
                "onboard_begin": "onboard/<wallet>/begin",
                "onboard_approval": "onboard/<wallet>/approval.json",
                "redeem_plan": "redeem/<wallet>/<slug>/plan.md",
                "redeem_review": "redeem/<wallet>/<slug>/review_intent.json",
                "redeem_approval": "redeem/<wallet>/<slug>/approval.json",
                "redeem_confirm": "redeem/<wallet>/<slug>/confirm",
                "redeem_receipt": "redeem/<wallet>/<slug>/receipt.json",
                "revoke_plan": "revoke-approvals/<wallet>/request/plan.md",
                "revoke_review": "revoke-approvals/<wallet>/request/review_intent.json",
                "revoke_approval": "revoke-approvals/<wallet>/request/approval.json",
                "revoke_confirm": "revoke-approvals/<wallet>/request/confirm",
                "revoke_receipt": "revoke-approvals/<wallet>/request/receipt.json",
                "withdraw_plan": "withdraw/<wallet>/pusd/plan.md",
                "withdraw_review": "withdraw/<wallet>/pusd/review_intent.json",
                "withdraw_approval": "withdraw/<wallet>/pusd/approval.json",
                "withdraw_confirm": "withdraw/<wallet>/pusd/confirm",
                "withdraw_receipt": "withdraw/<wallet>/pusd/receipt.json",
                "obligations": "obligations/<wallet>.json"
            },
            "generic_ipc_only": [
                "bloom:sign/signing@0.2.0",
                "bloom:tx/outbox@0.1.0",
                "bloom:chain/read@0.1.0"
            ]
        })),
        "parity.json" => read_json_value(&serde_json::json!({
            "kind": "polymarket_v2_petal_parity",
            "mount": "apps/polymarket",
            "status": "v2_implementation",
            "graduation_ready": true,
            "transaction_execution": "generic_bloom_tx_outbox_only",
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
                    "evidence": "live factory deposit-wallet resolution plus CLOB auth signature through generic sign_hash and private credential storage"
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
                    "id": "generic_deposit_wallet_exit_batches",
                    "surface": ["redeem/*/*/plan.md", "redeem/*/*/confirm", "revoke-approvals/*/request/plan.md", "revoke-approvals/*/request/confirm", "withdraw/*/pusd/plan.md", "withdraw/*/pusd/confirm"],
                    "evidence": "redeem, V2 approval revocation, and pUSD withdrawal persist and sign byte-exact deposit-wallet WALLET batches before relayer submission; retries verify the persisted EIP-712 preimage and resume polling rather than reconstructing a batch"
                },
                {
                    "id": "generic_outbox_funding",
                    "surface": ["settings/enso-api-key", "fund/*/new", "fund/*/*/review_intent.json", "fund/*/*/confirm"],
                    "evidence": "direct pUSD, native, and arbitrary ERC-20 funding persist exact review-bound transactions, use exact approvals plus Enso swaps where required, and reconcile origin-bound generic outbox receipts before advancing"
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
                "mocked signing and generic outbox lifecycle tests cover approval-required retry, exact prepared bytes, origin-bound outbox reuse, warning acknowledgement, and one-shot staging",
                "public VFS reads are swept for private CLOB credentials, builder credentials, API keys/passphrases, raw echoed signatures, raw CLOB response bodies, and echoed signature payloads",
                "adversarial review findings are fixed or documented in docs/reviews/2026-06-23-local-petal-plugins-closeout.md",
                "GTD order posting is rejected consistently with the current native surface"
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
        })),
        _ => error(-3, "not a meta file"),
    }
}

fn read_market(slug: &str, file: &str) -> DispatchResponse {
    let market: Market = match get_json(&format!("{GAMMA}/markets/slug/{slug}")) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    match file {
        "market.json" => read_json_value(&market),
        "book.json" => {
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
        "prices.json" => {
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
        _ => error(-3, "not a market file"),
    }
}

fn read_search(query: &str) -> DispatchResponse {
    let query = query.replace('+', " ");
    match get_json::<serde_json::Value>(&url_with_query(
        &format!("{GAMMA}/public-search"),
        &[("q", &query)],
    )) {
        Ok(value) => read_json_value(&value),
        Err(resp) => resp,
    }
}

fn read_positions(user: &str, file: &str) -> DispatchResponse {
    let user = match resolve_position_user(user) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    match file {
        "positions.json" => match get_json::<Vec<Position>>(&url_with_query(
            &format!("{DATA}/positions"),
            &[("user", &user)],
        )) {
            Ok(value) => read_json_value(&value),
            Err(resp) => resp,
        },
        "trades.json" => match get_json::<Vec<Trade>>(&url_with_query(
            &format!("{DATA}/trades"),
            &[("user", &user)],
        )) {
            Ok(value) => read_json_value(&value),
            Err(resp) => resp,
        },
        "activity.json" => match get_json::<serde_json::Value>(&url_with_query(
            &format!("{DATA}/activity"),
            &[("user", &user)],
        )) {
            Ok(value) => read_json_value(&value),
            Err(resp) => resp,
        },
        _ => error(-3, "not a positions file"),
    }
}

fn resolve_position_user(segment: &str) -> Result<String, DispatchResponse> {
    if (segment.starts_with("0x") || segment.starts_with("0X"))
        && let Ok(address) = segment.parse::<Address>()
    {
        return Ok(address.to_checksum(None));
    }
    wallet_address(segment).map(|address| address.to_checksum(None))
}

fn read_onboard(wallet: &str, file: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    match file {
        "begin" => DispatchResponse::Read(BEGIN_HINT.into()),
        "status.json" => {
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
        "plan.md" => DispatchResponse::Read(render_onboard_plan(wallet).into_bytes()),
        "approvals.json" => match wallet_address(wallet) {
            Ok(owner) => read_json_value(&approval_preview(wallet, owner)),
            Err(resp) => resp,
        },
        "approval.json" => read_store(&format!("onboard/{wallet}/approval.json")),
        "review_intent.json" => read_store(&format!("onboard/{wallet}/review_intent.json")),
        _ => error(-3, "not an onboard file"),
    }
}

fn read_account(wallet: &str, file: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    match file {
        "portfolio.json" => {
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
        "orders.json" => match clob_orders_for_wallet(wallet, owner) {
            Ok(orders) => read_json_value(&orders),
            Err(resp) => resp,
        },
        "status.json" => match local_status_for_wallet(wallet, owner) {
            Ok(status) => read_json_value(&account_status(wallet, owner, &status)),
            Err(resp) => resp,
        },
        "buying_power.json" => match account_buying_power(wallet, owner) {
            Ok(value) => read_json_value(&value),
            Err(resp) => resp,
        },
        "funding_options.json" => read_json_value(&serde_json::json!({
            "wallet": wallet,
            "target_asset": "pUSD",
            "options": [{
                "from": "pUSD",
                "supported": true,
                "review_required": true,
                "fund_route": format!("fund/{wallet}/new"),
                "execution": "generic_evm_outbox_direct_erc20_transfer"
            }, {
                "from": "native_or_other_erc20",
                "supported": true,
                "review_required": true,
                "fund_route": format!("fund/{wallet}/new"),
                "execution": "enso_quote_then_generic_evm_outbox",
                "enso_key_configured": load_enso_api_key().is_ok()
            }],
            "limits": {
                "policy_caps_apply": true,
                "requires_quote": true,
                "native_value_caps_are_quantity_caps": true
            }
        })),
        _ => error(-3, "not an account file"),
    }
}

fn account_status(wallet: &str, owner: Address, status: &serde_json::Value) -> serde_json::Value {
    let tradeable = status
        .get("tradeable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let stage = status
        .get("stage")
        .cloned()
        .unwrap_or(serde_json::Value::String("not_started".into()));
    serde_json::json!({
        "wallet": wallet,
        "owner_address": owner.to_checksum(None),
        "mode": "deposit_wallet",
        "deposit_wallet": status.get("deposit_wallet").cloned().unwrap_or(serde_json::Value::Null),
        "tradeable": tradeable,
        "onboarding_stage": stage,
        "credentials_present": status.get("creds_present").cloned().unwrap_or(serde_json::Value::Bool(false)),
        "next_required_action": if tradeable {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(format!("continue onboarding (stage: {})", status.get("stage").and_then(serde_json::Value::as_str).unwrap_or("not_started")))
        }
    })
}

fn account_buying_power(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    let creds = load_creds(wallet)?;
    let balance_allowance = clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    )?;
    let status = local_status_for_wallet(wallet, owner)?;
    let raw = balance_allowance
        .get("balance")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let has_balance = balance_allowance
        .get("balance")
        .and_then(parse_json_u256)
        .is_some_and(|balance| !balance.is_zero());
    let tradeable = status
        .get("tradeable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Ok(serde_json::json!({
        "wallet": wallet,
        "spendable": {
            "asset": "pUSD",
            "raw": raw,
            "source": "clob_balance_allowance",
            "clob_balance_allowance": balance_allowance,
        },
        "can_trade_now": tradeable && has_balance,
        "funding_needed": !has_balance,
        "funding_options_ref": format!("account/{wallet}/funding_options.json"),
        "notes": [
            "Spendable pUSD is sourced from the authenticated CLOB balance allowance.",
            "Native funding capacity is intentionally not included in this operational read."
        ]
    }))
}

fn clob_orders_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    let creds = load_creds(wallet)?;
    clob_l2_get_json(owner, &creds, "/data/orders", &[])
}

fn read_obligations(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let status = match local_status_for_wallet(wallet, owner) {
        Ok(status) => status,
        Err(resp) => return resp,
    };
    let Some(deposit) = fundable_deposit_wallet_from_status(&status) else {
        return error(
            -3,
            "deposit wallet is not factory-resolved; write onboard/<wallet>/begin before reading obligations",
        );
    };
    let positions = match get_json::<Vec<Position>>(&url_with_query(
        &format!("{DATA}/positions"),
        &[("user", &deposit.to_checksum(None))],
    )) {
        Ok(positions) => positions,
        Err(resp) => return resp,
    };
    let open: Vec<serde_json::Value> = positions
        .into_iter()
        .filter(|position| position.size.unwrap_or(0.0) > 0.0)
        .map(|position| {
            let receipt_ids = trade_receipt_ids_for_token(wallet, &position.asset);
            let next_action = if position.redeemable {
                "redeem_when_exit_workflow_is_available"
            } else {
                "sell_to_close_or_wait_for_redemption"
            };
            serde_json::json!({
                "title": position.title,
                "outcome": position.outcome,
                "token_id": position.asset,
                "condition_id": position.condition_id,
                "size": position.size,
                "avg_price": position.avg_price,
                "current_price": position.cur_price,
                "redeemable": position.redeemable,
                "bloom_receipts": receipt_ids,
                "next_action": next_action,
            })
        })
        .collect();
    read_json_value(&serde_json::json!({
        "wallet": wallet,
        "deposit_wallet": deposit.to_checksum(None),
        "tradeable": status.get("tradeable").cloned().unwrap_or(serde_json::Value::Bool(false)),
        "open_positions": open,
        "next_required_action": if open.is_empty() {
            "no_polymarket_exit_action_required"
        } else {
            "review_open_positions"
        }
    }))
}

fn trade_receipt_ids_for_token(wallet: &str, token_id: &str) -> Vec<String> {
    let prefix = format!("trade/{wallet}/receipts/");
    store_ids(&prefix, "/receipt.json")
        .into_iter()
        .filter(|id| {
            store_get(&format!("{prefix}{id}/receipt.json"))
                .and_then(|bytes| serde_json::from_slice::<StoreTradeReceipt>(&bytes).ok())
                .is_some_and(|receipt| receipt.token_id == token_id)
        })
        .collect()
}

fn read_builder_keys(wallet: &str, file: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    match file {
        "keys.json" => {
            let owner = match wallet_address(wallet) {
                Ok(owner) => owner,
                Err(resp) => return resp,
            };
            let creds = match load_creds(wallet) {
                Ok(creds) => creds,
                Err(resp) => return resp,
            };
            let value = match clob_l2_get_json(owner, &creds, "/auth/builder-api-key", &[]) {
                Ok(value) => value,
                Err(resp) => return resp,
            };
            let stored_key = match load_builder_credentials(wallet) {
                Ok(stored) => stored.map(|credentials| credentials.key),
                Err(resp) => return resp,
            };
            let keys: Vec<serde_json::Value> = builder_key_infos(&value)
                .into_iter()
                .map(|key| {
                    serde_json::json!({
                        "key": key.key,
                        "created_at": key.created_at,
                        "revoked_at": key.revoked_at,
                        "stored_by_petal": stored_key.as_deref() == Some(key.key.as_str()),
                    })
                })
                .collect();
            read_json_value(&serde_json::json!({
                "wallet": wallet,
                "keys": keys,
                "secrets_exposed": false,
            }))
        }
        "revoke" => DispatchResponse::Read(BUILDER_KEY_REVOKE_HINT.into()),
        _ => error(-3, "not a builder-key file"),
    }
}

fn write_builder_key_revoke(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let key = match parse_builder_key_revoke(body) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let stored = match load_builder_credentials(wallet) {
        Ok(stored) => stored,
        Err(resp) => return resp,
    };
    let body = key
        .as_deref()
        .map(|key| serde_json::json!({ "key": key }).to_string())
        .unwrap_or_default();
    if let Err(resp) = clob_l2_delete_json(owner, &creds, "/auth/builder-api-key", &body) {
        return resp;
    }
    if stored
        .as_ref()
        .is_some_and(|stored| key.is_none() || key.as_deref() == Some(stored.key.as_str()))
        && let Err(resp) = delete_builder_credentials(wallet)
    {
        return resp;
    }
    DispatchResponse::Write
}

fn builder_key_infos(value: &serde_json::Value) -> Vec<bloom_polymarket::BuilderApiKeyInfo> {
    let entries = value
        .as_array()
        .or_else(|| value.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| value.get("keys").and_then(serde_json::Value::as_array));
    entries
        .into_iter()
        .flatten()
        .filter_map(bloom_polymarket::BuilderApiKeyInfo::from_value)
        .filter(|info| !info.key.trim().is_empty())
        .collect()
}

fn parse_builder_key_revoke(body: &[u8]) -> Result<Option<String>, DispatchResponse> {
    let text = core::str::from_utf8(body)
        .map_err(|_| error(-3, "builder-key revoke body must be UTF-8"))?
        .trim();
    if matches!(text.to_ascii_lowercase().as_str(), "confirm" | "y" | "yes") {
        return Ok(None);
    }
    let request: BuilderKeyRevokeRequest = serde_json::from_str(text)
        .map_err(|e| error(-3, format!("builder-key revoke JSON: {e}")))?;
    if !request.confirm {
        return Err(error(-3, "builder-key revoke must set confirm=true"));
    }
    if let Some(key) = request.key {
        if !is_safe_external_id(&key) {
            return Err(error(-3, "invalid builder-key id"));
        }
        Ok(Some(key))
    } else {
        Ok(None)
    }
}

fn write_enso_api_key(body: &[u8]) -> DispatchResponse {
    let key = match core::str::from_utf8(body) {
        Ok(value) => value.trim(),
        Err(_) => return error(-3, "Enso API key must be UTF-8"),
    };
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_whitespace) {
        return error(-3, "Enso API key must be 1-4096 non-whitespace characters");
    }
    match bloom_petal_sdk::store_put("creds/enso-api-key", key.as_bytes(), true) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

fn load_enso_api_key() -> Result<String, DispatchResponse> {
    let bytes = bloom_petal_sdk::store_get("creds/enso-api-key", 4096).map_err(|e| match e {
        SdkError::Host(HostStatus::NotFound) => error(
            -3,
            "Enso API key is not configured; write it to settings/enso-api-key",
        ),
        other => sdk_error(other),
    })?;
    String::from_utf8(bytes).map_err(|_| error(-4, "stored Enso API key is not valid UTF-8"))
}

fn write_onboard_begin(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let deposit = match predict_deposit_wallet(owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(DispatchResponse::Error { code: -3, .. }) => {
            let prepared_key = format!("onboard/{wallet}/prepared_clob_auth.json");
            let approval_key = format!("onboard/{wallet}/approval.json");
            let prepared = match load_prepared_signing(&prepared_key) {
                Ok(Some(PreparedSigning::ClobAuth(prepared))) => {
                    if prepared.owner != owner.to_checksum(None)
                        || prepared.nonce != CLOB_AUTH_NONCE
                        || prepared.chain_id != POLYGON
                        || prepared.credential_action != "mint_or_derive"
                    {
                        return error(
                            -4,
                            "prepared CLOB auth does not match this onboarding request",
                        );
                    }
                    PreparedSigning::ClobAuth(prepared)
                }
                Ok(Some(_)) => {
                    return error(-4, "unexpected prepared signing operation for onboarding");
                }
                Ok(None) => {
                    let timestamp = now_secs();
                    let hash = clob_auth_signing_hash(owner, timestamp, CLOB_AUTH_NONCE, POLYGON);
                    let review_intent = serde_json::json!({
                        "operation": "clob_auth",
                        "wallet": wallet,
                        "owner": owner.to_checksum(None),
                        "nonce": CLOB_AUTH_NONCE,
                        "timestamp": timestamp,
                        "credential_action": "mint_or_derive",
                        "chain_id": POLYGON,
                        "signing_hash": format!("{hash:#x}"),
                    });
                    let review_intent_hash = match store_review_intent(
                        &format!("onboard/{wallet}/review_intent.json"),
                        &review_intent,
                    ) {
                        Ok(hash) => hash,
                        Err(response) => return response,
                    };
                    let prepared = PreparedSigning::ClobAuth(PreparedClobAuth {
                        owner: owner.to_checksum(None),
                        nonce: CLOB_AUTH_NONCE,
                        timestamp,
                        credential_action: "mint_or_derive".into(),
                        chain_id: POLYGON,
                        signing_hash: format!("{hash:#x}"),
                        review_intent_hash,
                    });
                    if let Err(response) = store_prepared_signing(&prepared_key, &prepared) {
                        return response;
                    }
                    prepared
                }
                Err(response) => return response,
            };
            let PreparedSigning::ClobAuth(auth) = &prepared else {
                return error(-4, "unexpected prepared signing operation for onboarding");
            };
            let expected_hash =
                clob_auth_signing_hash(owner, auth.timestamp, auth.nonce, auth.chain_id);
            if prepared.signing_hash().ok() != Some(expected_hash) {
                return error(-4, "prepared CLOB auth hash does not match its preimage");
            }
            if let Err(response) = verify_review_intent(
                &format!("onboard/{wallet}/review_intent.json"),
                &auth.review_intent_hash,
            ) {
                return response;
            }
            let signature = match sign_prepared(wallet, &prepared, &approval_key) {
                Ok(signature) => format!("0x{}", hex::encode(signature)),
                Err(response) => return response,
            };
            let headers = [
                (POLY_ADDRESS, format!("{owner:#x}")),
                (POLY_NONCE, auth.nonce.to_string()),
                (POLY_SIGNATURE, signature),
                (POLY_TIMESTAMP, auth.timestamp.to_string()),
            ];
            let creds = match clob_auth_request("POST", "/auth/api-key", &headers) {
                Ok(creds) => creds,
                Err(err)
                    if err.status.is_some_and(|status| {
                        (400..500).contains(&status) && !matches!(status, 401 | 403 | 429)
                    }) =>
                {
                    match clob_auth_request("GET", "/auth/derive-api-key", &headers) {
                        Ok(creds) => creds,
                        Err(err) => return err.response,
                    }
                }
                Err(err) => return err.response,
            };
            if let DispatchResponse::Error { .. } =
                store_put_json(&format!("creds/{wallet}/clob.json"), &creds, true)
            {
                return error(-4, "failed to store CLOB credentials");
            }
            let _ = bloom_petal_sdk::store_del(&prepared_key);
            let _ = bloom_petal_sdk::store_del(&approval_key);
            creds
        }
        Err(response) => return response,
    };
    match run_onboard_stages(wallet, owner, deposit, &creds) {
        Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
        Err(resp) => {
            let _ = persist_onboard_failure(wallet, owner, deposit, &resp);
            resp
        }
    }
}

fn local_onboard_status(
    wallet: &str,
    owner: Address,
    stage: &str,
    running: bool,
    creds_present: bool,
    message: &str,
) -> serde_json::Value {
    let deposit = derive_deposit_wallet_address(&owner, POLYGON);
    serde_json::json!({
        "wallet": wallet,
        "owner": format!("{owner:#x}"),
        "stage": stage,
        "running": running,
        "tradeable": false,
        "creds_present": creds_present,
        "deposit_wallet": {
            "address": deposit.to_checksum(None),
            "source": "local_estimate_unverified",
            "fundable": false,
            "warning": "do not fund this local estimate; full onboarding must resolve the live factory address first"
        },
        "approvals": {
            "required": true,
            "preview_path": format!("onboard/{wallet}/approvals.json")
        },
        "message": message
    })
}

struct LiveOnboardStatus<'a> {
    wallet: &'a str,
    owner: Address,
    deposit: Address,
    stage: &'a str,
    running: bool,
    creds_present: bool,
    tradeable: bool,
    message: &'a str,
    probes: serde_json::Value,
}

fn local_onboard_status_with_live_deposit(status: LiveOnboardStatus<'_>) -> serde_json::Value {
    serde_json::json!({
        "wallet": status.wallet,
        "owner": status.owner.to_checksum(None),
        "stage": status.stage,
        "running": status.running,
        "tradeable": status.tradeable,
        "creds_present": status.creds_present,
        "deposit_wallet": {
            "address": status.deposit.to_checksum(None),
            "source": "live_factory_resolved",
            "fundable": true,
            "warning": serde_json::Value::Null
        },
        "approvals": {
            "required": true,
            "preview_path": format!("onboard/{}/approvals.json", status.wallet)
        },
        "probes": status.probes,
        "message": status.message
    })
}

fn refreshed_live_onboard_status(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds_present: bool,
) -> Result<serde_json::Value, DispatchResponse> {
    let deployed = read_chain_deposit_wallet_deployed(deposit)?;
    let pusd_balance = if deployed {
        read_chain_erc20_balance(PUSD, deposit)?
    } else {
        U256::ZERO
    };
    let approvals_in_place = if deployed && !pusd_balance.is_zero() {
        read_chain_v2_approvals(deposit)?
    } else {
        false
    };
    let (clob_synced, clob_balance, clob_allowance) =
        if deployed && !pusd_balance.is_zero() && approvals_in_place && creds_present {
            let creds = load_creds(wallet)?;
            read_clob_collateral_sync(owner, &creds)?
        } else {
            (false, None, None)
        };

    let (stage, tradeable, message) = if !deployed {
        (
            "deploy",
            false,
            "deposit wallet resolved from the live factory; waiting for the native relayer deploy stage",
        )
    } else if pusd_balance.is_zero() {
        (
            "fund",
            false,
            "deposit wallet is deployed; waiting for pUSD funding",
        )
    } else if !approvals_in_place {
        (
            "approve",
            false,
            "deposit wallet holds pUSD; waiting for V2 exchange and adapter approvals",
        )
    } else if !creds_present {
        (
            "creds",
            false,
            "deposit wallet is funded and approved; write begin to mint or derive CLOB credentials",
        )
    } else if !clob_synced {
        (
            "sync",
            false,
            "deposit wallet is funded and approved; waiting for CLOB collateral balance/allowance sync",
        )
    } else {
        (
            "complete",
            true,
            "local read-only probes show the deposit wallet is deployed, funded, approved, credentialed, and CLOB-synced",
        )
    };

    Ok(local_onboard_status_with_live_deposit(LiveOnboardStatus {
        wallet,
        owner,
        deposit,
        stage,
        running: false,
        creds_present,
        tradeable,
        message,
        probes: serde_json::json!({
            "source": "vfs_chain_and_clob_read_only",
            "deposit_wallet_deployed": deployed,
            "pusd_balance_raw": pusd_balance.to_string(),
            "approvals_in_place": approvals_in_place,
            "clob_collateral_balance_raw": clob_balance.map(|v| v.to_string()),
            "clob_collateral_allowance_raw": clob_allowance.map(|v| v.to_string()),
            "clob_collateral_synced": clob_synced
        }),
    }))
}

fn run_onboard_stages(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds: &Credentials,
) -> Result<serde_json::Value, DispatchResponse> {
    let mut deploy_tx_id = stored_status_for_wallet(wallet, owner)
        .ok()
        .and_then(|status| {
            status
                .get("deploy_tx_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let mut approve_tx_id = stored_status_for_wallet(wallet, owner)
        .ok()
        .and_then(|status| {
            status
                .get("approve_tx_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });

    if !read_chain_deposit_wallet_deployed(deposit)? {
        let _builder = ensure_builder_credentials(wallet, owner, creds)?;
        let tx = onboard_deploy_submission(wallet, owner, creds, deploy_tx_id.as_deref())?;
        deploy_tx_id = Some(tx.id.clone());
        persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("deploy"),
                deploy_tx_id: deploy_tx_id.clone(),
                approve_tx_id: approve_tx_id.clone(),
                in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        )?;
        let confirmed = match relayer_poll_once(&tx) {
            Ok(RelayerPoll::Confirmed(done)) => done,
            Ok(RelayerPoll::Pending(pending)) => {
                persist_relayer_progress_identity(
                    &format!("onboard/{wallet}/deploy_submission_started.json"),
                    &pending,
                )?;
                return persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("deploy"),
                        deploy_tx_id: Some(pending.id),
                        approve_tx_id,
                        in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                        relayer_auth: Some("builder_key_auto"),
                        last_error: None,
                    },
                );
            }
            Ok(RelayerPoll::Failed(failed)) => {
                let _ = bloom_petal_sdk::store_del(&format!(
                    "onboard/{wallet}/deploy_submission_started.json"
                ));
                return Err(error(
                    -4,
                    format!(
                        "relayer deploy transaction {} failed in state {}; retry to prepare a fresh submission",
                        failed.id, failed.state
                    ),
                ));
            }
            Err(resp) => {
                let msg = dispatch_error_message(&resp);
                persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("deploy"),
                        deploy_tx_id,
                        approve_tx_id,
                        in_flight_deadline_ms: None,
                        relayer_auth: Some("builder_key_auto"),
                        last_error: Some(msg),
                    },
                )?;
                return Err(resp);
            }
        };
        deploy_tx_id = Some(confirmed.id);
        let _ =
            bloom_petal_sdk::store_del(&format!("onboard/{wallet}/deploy_submission_started.json"));
        if !read_chain_deposit_wallet_deployed(deposit)? {
            let msg = "relayer confirmed the deploy but no proxy implementation exists at the deposit wallet".to_string();
            persist_onboard_status(
                wallet,
                owner,
                deposit,
                true,
                OnboardStatusExtra {
                    stage: Some("deploy"),
                    deploy_tx_id,
                    approve_tx_id,
                    in_flight_deadline_ms: None,
                    relayer_auth: Some("builder_key_auto"),
                    last_error: Some(msg.clone()),
                },
            )?;
            return Err(error(-4, msg));
        }
    }

    let pusd_balance = read_chain_erc20_balance(PUSD, deposit)?;
    if pusd_balance.is_zero() {
        return persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("fund"),
                deploy_tx_id,
                approve_tx_id,
                in_flight_deadline_ms: None,
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        );
    }

    if !read_chain_v2_approvals(deposit)? {
        let _builder = ensure_builder_credentials(wallet, owner, creds)?;
        let tx =
            onboard_approval_submission(wallet, owner, deposit, creds, approve_tx_id.as_deref())?;
        approve_tx_id = Some(tx.id.clone());
        persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("approve"),
                deploy_tx_id: deploy_tx_id.clone(),
                approve_tx_id: approve_tx_id.clone(),
                in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        )?;
        let confirmed = match relayer_poll_once(&tx) {
            Ok(RelayerPoll::Confirmed(done)) => done,
            Ok(RelayerPoll::Pending(pending)) => {
                persist_relayer_progress_identity(
                    &format!("onboard/{wallet}/approval_submission_started.json"),
                    &pending,
                )?;
                return persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("approve"),
                        deploy_tx_id,
                        approve_tx_id: Some(pending.id),
                        in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                        relayer_auth: Some("builder_key_auto"),
                        last_error: None,
                    },
                );
            }
            Ok(RelayerPoll::Failed(failed)) => {
                let _ = bloom_petal_sdk::store_del(&format!(
                    "onboard/{wallet}/approval_submission_started.json"
                ));
                let _ = bloom_petal_sdk::store_del(&format!(
                    "onboard/{wallet}/prepared_relayer_batch.json"
                ));
                let _ = bloom_petal_sdk::store_del(&format!("onboard/{wallet}/approval.json"));
                return Err(error(
                    -4,
                    format!(
                        "relayer approval transaction {} failed in state {}; retry to prepare a fresh batch",
                        failed.id, failed.state
                    ),
                ));
            }
            Err(resp) => {
                let msg = dispatch_error_message(&resp);
                persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("approve"),
                        deploy_tx_id,
                        approve_tx_id,
                        in_flight_deadline_ms: None,
                        relayer_auth: Some("builder_key_auto"),
                        last_error: Some(msg),
                    },
                )?;
                return Err(resp);
            }
        };
        approve_tx_id = Some(confirmed.id);
        let _ = bloom_petal_sdk::store_del(&format!(
            "onboard/{wallet}/approval_submission_started.json"
        ));
        let _ =
            bloom_petal_sdk::store_del(&format!("onboard/{wallet}/prepared_relayer_batch.json"));
        let _ = bloom_petal_sdk::store_del(&format!("onboard/{wallet}/approval.json"));
        if !read_chain_v2_approvals(deposit)? {
            let msg = "approvals confirmed but on-chain allowances are still missing".to_string();
            persist_onboard_status(
                wallet,
                owner,
                deposit,
                true,
                OnboardStatusExtra {
                    stage: Some("approve"),
                    deploy_tx_id,
                    approve_tx_id,
                    in_flight_deadline_ms: None,
                    relayer_auth: Some("builder_key_auto"),
                    last_error: Some(msg.clone()),
                },
            )?;
            return Err(error(-4, msg));
        }
    }

    persist_onboard_status(
        wallet,
        owner,
        deposit,
        true,
        OnboardStatusExtra {
            stage: Some("sync"),
            deploy_tx_id: deploy_tx_id.clone(),
            approve_tx_id: approve_tx_id.clone(),
            in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
            relayer_auth: Some("builder_key_auto"),
            last_error: None,
        },
    )?;
    clob_l2_get_json(
        owner,
        creds,
        "/balance-allowance/update",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    )?;
    persist_onboard_status(
        wallet,
        owner,
        deposit,
        true,
        OnboardStatusExtra {
            stage: None,
            deploy_tx_id,
            approve_tx_id,
            in_flight_deadline_ms: None,
            relayer_auth: Some("builder_key_auto"),
            last_error: None,
        },
    )
}

fn onboard_deploy_submission(
    wallet: &str,
    owner: Address,
    creds: &Credentials,
    existing_tx_id: Option<&str>,
) -> Result<LocalRelayerTx, DispatchResponse> {
    let marker_key = format!("onboard/{wallet}/deploy_submission_started.json");
    if let Some(id) = existing_tx_id {
        let progress = load_relayer_action_progress(&marker_key)?;
        return resume_relayer_transaction(id, progress.as_ref());
    }
    if let Some(bytes) = store_get(&marker_key) {
        let marker: RelayerActionProgress = serde_json::from_slice(&bytes)
            .map_err(|e| error(-4, format!("corrupt deployment progress: {e}")))?;
        if let Some(id) = marker.transaction_id.clone() {
            return resume_relayer_transaction(&id, Some(&marker));
        }
        return Err(error(
            -4,
            "deposit-wallet deployment may have been accepted without returning a transaction id; refusing automatic resubmission",
        ));
    }
    let digest = blake3_hex(
        format!(
            "deposit_wallet_deploy:{}:{}",
            owner.to_checksum(None),
            FACTORY.to_checksum(None)
        )
        .as_bytes(),
    );
    let marker = RelayerActionProgress {
        prepared_artifact_digest: digest.clone(),
        phase: "submission_started".into(),
        transaction_id: None,
        relayer_state: None,
        transaction_hash: None,
    };
    if let DispatchResponse::Error { .. } = store_put_json(&marker_key, &marker, false) {
        return Err(error(-4, "failed to persist deployment submission marker"));
    }
    let tx = match relayer_submit_with_builder_repair_classified(
        wallet,
        owner,
        creds,
        serde_json::json!({
            "type": "WALLET-CREATE",
            "from": owner.to_checksum(None),
            "to": FACTORY.to_checksum(None),
        }),
    ) {
        Ok(tx) => tx,
        Err(failure) => {
            if !failure.ambiguous {
                let _ = bloom_petal_sdk::store_del(&marker_key);
            }
            return Err(failure.response);
        }
    };
    let submitted = RelayerActionProgress {
        prepared_artifact_digest: digest,
        phase: "submitted".into(),
        transaction_id: Some(tx.id.clone()),
        relayer_state: Some(tx.state.clone()),
        transaction_hash: tx.transaction_hash.clone(),
    };
    if let DispatchResponse::Error { .. } = store_put_json(&marker_key, &submitted, false) {
        return Err(error(
            -4,
            "deployment transaction id could not be persisted",
        ));
    }
    Ok(tx)
}

fn onboard_approval_submission(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds: &Credentials,
    existing_tx_id: Option<&str>,
) -> Result<LocalRelayerTx, DispatchResponse> {
    let marker_key = format!("onboard/{wallet}/approval_submission_started.json");
    if let Some(id) = existing_tx_id {
        let progress = load_relayer_action_progress(&marker_key)?;
        return resume_relayer_transaction(id, progress.as_ref());
    }
    if let Some(progress) = load_relayer_action_progress(&marker_key)? {
        if let Some(id) = progress.transaction_id.clone() {
            return resume_relayer_transaction(&id, Some(&progress));
        }
        return Err(error(
            -4,
            "approval batch may have been accepted without returning a transaction id; refusing to sign or submit it again",
        ));
    }
    let nonce = relayer_wallet_nonce(owner)?;
    let deadline = now_secs().saturating_add(BATCH_DEADLINE_SECS);
    let body = relayer_batch_body(wallet, owner, deposit, nonce, deadline)?;
    let prepared = load_prepared_signing(&format!("onboard/{wallet}/prepared_relayer_batch.json"))?
        .ok_or_else(|| error(-4, "missing prepared onboarding approval batch"))?;
    let prepared_artifact_digest = prepared_digest(&prepared)?;
    let marker = RelayerActionProgress {
        prepared_artifact_digest: prepared_artifact_digest.clone(),
        phase: "submission_started".into(),
        transaction_id: None,
        relayer_state: None,
        transaction_hash: None,
    };
    if let DispatchResponse::Error { .. } = store_put_json(&marker_key, &marker, false) {
        return Err(error(-4, "failed to persist approval submission marker"));
    }
    let tx = match relayer_submit_with_builder_repair_classified(wallet, owner, creds, body) {
        Ok(tx) => tx,
        Err(failure) => {
            if !failure.ambiguous {
                let _ = bloom_petal_sdk::store_del(&marker_key);
            }
            return Err(failure.response);
        }
    };
    let submitted = RelayerActionProgress {
        prepared_artifact_digest,
        phase: "submitted".into(),
        transaction_id: Some(tx.id.clone()),
        relayer_state: Some(tx.state.clone()),
        transaction_hash: tx.transaction_hash.clone(),
    };
    if let DispatchResponse::Error { .. } = store_put_json(&marker_key, &submitted, false) {
        return Err(error(-4, "approval transaction id could not be persisted"));
    }
    Ok(tx)
}

#[derive(Default)]
struct OnboardStatusExtra {
    stage: Option<&'static str>,
    deploy_tx_id: Option<String>,
    approve_tx_id: Option<String>,
    in_flight_deadline_ms: Option<u128>,
    relayer_auth: Option<&'static str>,
    last_error: Option<String>,
}

fn persist_onboard_status(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds_present: bool,
    extra: OnboardStatusExtra,
) -> Result<serde_json::Value, DispatchResponse> {
    let mut status = refreshed_live_onboard_status(wallet, owner, deposit, creds_present)?;
    if let Some(obj) = status.as_object_mut() {
        if let Some(stage) = extra.stage {
            obj.insert("stage".into(), serde_json::Value::String(stage.into()));
            obj.insert(
                "tradeable".into(),
                serde_json::Value::Bool(stage == "complete"),
            );
        }
        obj.insert(
            "deploy_tx_id".into(),
            extra
                .deploy_tx_id
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "approve_tx_id".into(),
            extra
                .approve_tx_id
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "in_flight_deadline_ms".into(),
            extra
                .in_flight_deadline_ms
                .map(|v| serde_json::Value::String(v.to_string()))
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "relayer_auth".into(),
            extra
                .relayer_auth
                .map(|v| serde_json::Value::String(v.into()))
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "last_error".into(),
            extra
                .last_error
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "status_updated_ms".into(),
            serde_json::Value::String(now_millis().to_string()),
        );
    }
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("onboard/{wallet}/status.json"), &status, false)
    {
        return Err(error(-4, "failed to persist onboarding status"));
    }
    Ok(status)
}

fn persist_onboard_failure(
    wallet: &str,
    owner: Address,
    deposit: Address,
    resp: &DispatchResponse,
) -> Result<serde_json::Value, DispatchResponse> {
    persist_onboard_status(
        wallet,
        owner,
        deposit,
        store_get(&format!("creds/{wallet}/clob.json")).is_some(),
        OnboardStatusExtra {
            last_error: Some(dispatch_error_message(resp)),
            ..OnboardStatusExtra::default()
        },
    )
}

fn local_status_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    let status = stored_status_for_wallet(wallet, owner)?;
    let deposit_value = status.get("deposit_wallet");
    let source = deposit_value
        .and_then(|value| value.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if source != "live_factory_resolved" {
        return Ok(status);
    }
    let deposit = deposit_value
        .and_then(|value| value.get("address"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "stored onboarding status is missing deposit wallet"))?
        .parse::<Address>()
        .map_err(|e| error(-4, format!("stored deposit wallet parse: {e}")))?;
    let creds_present = store_get(&format!("creds/{wallet}/clob.json")).is_some();
    let mut refreshed = refreshed_live_onboard_status(wallet, owner, deposit, creds_present)?;
    preserve_onboard_metadata(&status, &mut refreshed);
    if refreshed != status
        && let DispatchResponse::Error { .. } =
            store_put_json(&format!("onboard/{wallet}/status.json"), &refreshed, false)
    {
        return Err(error(-4, "failed to refresh onboarding status"));
    }
    Ok(refreshed)
}

fn preserve_onboard_metadata(previous: &serde_json::Value, refreshed: &mut serde_json::Value) {
    let refreshed_complete =
        refreshed.get("stage").and_then(serde_json::Value::as_str) == Some("complete");
    let Some(obj) = refreshed.as_object_mut() else {
        return;
    };
    for key in [
        "deploy_tx_id",
        "approve_tx_id",
        "relayer_auth",
        "status_updated_ms",
    ] {
        if let Some(value) = previous.get(key) {
            obj.insert(key.into(), value.clone());
        }
    }
    if previous
        .get("in_flight_deadline_ms")
        .is_some_and(|value| !value.is_null())
        && !refreshed_complete
    {
        obj.insert(
            "in_flight_deadline_ms".into(),
            previous["in_flight_deadline_ms"].clone(),
        );
    }
    if previous
        .get("last_error")
        .is_some_and(|value| !value.is_null())
        && !refreshed_complete
    {
        obj.insert("last_error".into(), previous["last_error"].clone());
    }
}

fn stored_status_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    match store_get(&format!("onboard/{wallet}/status.json"))
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    {
        Some(status) => {
            let expected = owner.to_checksum(None);
            let Some(stored_owner) = status.get("owner").and_then(serde_json::Value::as_str) else {
                return Err(error(-4, "stored onboarding status is missing owner"));
            };
            let stored_owner = stored_owner
                .parse::<Address>()
                .map_err(|e| error(-4, format!("stored onboarding owner parse: {e}")))?
                .to_checksum(None);
            if stored_owner != expected {
                return Err(error(
                    -3,
                    "stored onboarding status belongs to a different wallet owner",
                ));
            }
            Ok(status)
        }
        None => Ok(local_onboard_status(
            wallet,
            owner,
            "not_started",
            false,
            false,
            "write begin to mint or derive CLOB credentials",
        )),
    }
}

fn fundable_deposit_wallet(wallet: &str, owner: Address) -> Result<Address, DispatchResponse> {
    let status = stored_status_for_wallet(wallet, owner)?;
    fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; write onboard/<wallet>/begin before funding",
        )
    })
}

fn tradeable_deposit_wallet(wallet: &str, owner: Address) -> Result<Address, DispatchResponse> {
    let status = local_status_for_wallet(wallet, owner)?;
    let deposit = fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; write onboard/<wallet>/begin before posting",
        )
    })?;
    if status.get("stage").and_then(serde_json::Value::as_str) != Some("complete")
        || !status
            .get("tradeable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    {
        return Err(error(
            -3,
            "wallet onboarding is not complete; read onboard/<wallet>/status.json and complete deploy, fund, approve, credentials, and CLOB sync before posting",
        ));
    }
    Ok(deposit)
}

fn fundable_deposit_wallet_from_status(status: &serde_json::Value) -> Option<Address> {
    let deposit = status
        .get("deposit_wallet")
        .and_then(|value| value.get("address"))
        .and_then(serde_json::Value::as_str)?;
    let source = status
        .get("deposit_wallet")
        .and_then(|value| value.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let fundable = status
        .get("deposit_wallet")
        .and_then(|value| value.get("fundable"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if source != "live_factory_resolved" || !fundable {
        return None;
    }
    deposit.parse::<Address>().ok()
}

fn approval_preview(wallet: &str, owner: Address) -> serde_json::Value {
    let status = stored_status_for_wallet(wallet, owner).ok();
    let deposit = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("address"))
                .and_then(serde_json::Value::as_str)
        })
        .and_then(|address| address.parse::<Address>().ok())
        .unwrap_or_else(|| derive_deposit_wallet_address(&owner, POLYGON));
    let deposit_source = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("source"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("local_estimate_unverified");
    let raw_deposit_fundable = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("fundable"))
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false);
    let deposit_fundable = raw_deposit_fundable && deposit_source == "live_factory_resolved";
    let calls: Vec<serde_json::Value> = v2_approval_calls()
        .iter()
        .zip(V2_APPROVAL_LABELS)
        .map(|(call, label)| {
            serde_json::json!({
                "label": label,
                "target": format!("{:#x}", call.target),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(call.data.as_ref())),
            })
        })
        .collect();
    serde_json::json!({
        "wallet": wallet,
        "owner": format!("{owner:#x}"),
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": deposit_source,
        "deposit_wallet_fundable": deposit_fundable,
        "warning": if deposit_fundable {
            serde_json::Value::Null
        } else {
            serde_json::Value::String("do not fund this locally derived estimate; full onboarding must resolve the live factory address before funding or approvals".into())
        },
        "chain_id": POLYGON,
        "calls": calls,
        "signing": "preview_only"
    })
}

fn write_trade_new(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let req: TradeNewRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("trade new JSON: {e}")),
    };
    let side = match req
        .side
        .as_deref()
        .unwrap_or("buy")
        .to_ascii_lowercase()
        .as_str()
    {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        other => return error(-3, format!("side must be buy or sell, got {other}")),
    };
    let amount_micro = match parse_micro(req.amount.trim()) {
        Ok(value) if value > 0 => value,
        Ok(_) => return error(-3, "amount must be > 0"),
        Err(e) => return error(-3, e.to_string()),
    };
    let bound = match side {
        Side::Buy => req.max_price.as_ref().or(req.limit_price.as_ref()),
        Side::Sell => req.min_price.as_ref().or(req.limit_price.as_ref()),
    };
    let Some(bound) = bound else {
        return error(
            -3,
            match side {
                Side::Buy => "buy requires max_price or limit_price",
                Side::Sell => "sell requires min_price or limit_price",
            },
        );
    };
    let bound_micro = match parse_micro(bound.trim()) {
        Ok(value) if value > 0 => value,
        Ok(_) => return error(-3, "price bound must be > 0"),
        Err(e) => return error(-3, e.to_string()),
    };
    let order_type = match req.order_type.as_deref() {
        Some(raw) => match raw.parse::<OrderType>() {
            Ok(OrderType::GTD) => return error(-3, "GTD orders are not supported"),
            Ok(value) => value,
            Err(e) => return error(-3, e.to_string()),
        },
        None if req.limit_price.is_some() => OrderType::GTC,
        None => OrderType::FAK,
    };
    let snapshot = match trade_snapshot(&req.slug, &req.outcome) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let marketable = req.limit_price.is_none();
    let pinned_limit_micro = match req.limit_price.as_deref() {
        Some(limit) => match parse_micro(limit.trim()) {
            Ok(value) if value > 0 => value,
            Ok(_) => return error(-3, "limit_price must be > 0"),
            Err(e) => return error(-3, e.to_string()),
        },
        None => bound_micro,
    };
    if !marketable {
        match side {
            Side::Buy if pinned_limit_micro > bound_micro => {
                return error(-3, "limit_price exceeds max_price");
            }
            Side::Sell if pinned_limit_micro < bound_micro => {
                return error(-3, "limit_price is below min_price");
            }
            _ => {}
        }
    }
    let limit_micro =
        match choose_trade_limit(side, marketable, bound_micro, pinned_limit_micro, &snapshot) {
            Ok(limit) => limit,
            Err(resp) => return resp,
        };
    let quote = match build_trade_quote(side, amount_micro, limit_micro, &snapshot, order_type) {
        Ok(quote) => quote,
        Err(resp) => return resp,
    };
    let id = next_id(&format!("trade/{wallet}/drafts/"), "/order.json");
    let draft = StoreTradeDraft {
        id: id.clone(),
        wallet: wallet.into(),
        slug: req.slug,
        question: snapshot.market.question,
        condition_id: snapshot.market.condition_id,
        outcome: snapshot.outcome,
        token_id: snapshot.token_id,
        side,
        order_type,
        amount_micro,
        price_bound_micro: bound_micro,
        limit_price: req.limit_price,
        marketable,
        limit_price_micro: quote.price_micro,
        size_micro: quote.size_micro,
        maker_micro: quote.maker_micro,
        taker_micro: quote.taker_micro,
        tick_micro: snapshot.tick_micro,
        min_order_size_micro: snapshot.min_size_micro,
        neg_risk: snapshot.neg_risk,
        active: snapshot.active,
        closed: snapshot.closed,
        order_book_enabled: snapshot.order_book_enabled,
        binary_outcomes: true,
        best_ask_micro: snapshot.best_ask_micro,
        best_bid_micro: snapshot.best_bid_micro,
        book_snapshot_secs: now_secs(),
        status: "review".into(),
        salt: None,
        clob_order_id: None,
        clob_status: None,
        last_error: None,
    };
    let policy_check = match trade_policy_check(wallet, &draft) {
        Ok(check) => check,
        Err(resp) => return resp,
    };
    let base = format!("trade/{wallet}/drafts/{id}");
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/order.json"), &draft, false)
    {
        return error(-4, "failed to store draft");
    }
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return error(-4, "failed to store policy check");
    }
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/quote.json"),
        &serde_json::json!({
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "marketable": draft.marketable,
            "amount_micro": draft.amount_micro,
            "amount": format_micro(draft.amount_micro),
            "price_bound_micro": draft.price_bound_micro,
            "price_bound": format_micro(draft.price_bound_micro),
            "limit_price_micro": draft.limit_price_micro,
            "limit_price": format_micro(draft.limit_price_micro),
            "size_micro": draft.size_micro,
            "size": format_micro(draft.size_micro),
            "maker_micro": draft.maker_micro,
            "maker": format_micro(draft.maker_micro),
            "taker_micro": draft.taker_micro,
            "taker": format_micro(draft.taker_micro),
            "tick_micro": draft.tick_micro,
            "tick": format_micro(draft.tick_micro),
            "min_order_size_micro": draft.min_order_size_micro,
            "min_order_size": format_micro(draft.min_order_size_micro),
            "best_ask_micro": draft.best_ask_micro,
            "best_bid_micro": draft.best_bid_micro,
            "status": "quoted"
        }),
        false,
    ) {
        return error(-4, "failed to store quote");
    }
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/review_intent.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "slug": draft.slug,
            "outcome": draft.outcome,
            "token_id": draft.token_id,
            "limit_price": format_micro(draft.limit_price_micro),
            "size": format_micro(draft.size_micro),
            "status": "created"
        }),
        false,
    ) {
        return error(-4, "failed to store review intent");
    }
    DispatchResponse::Write
}

fn trade_snapshot(slug: &str, outcome: &str) -> Result<TradeSnapshot, DispatchResponse> {
    let market: Market = get_json(&format!("{GAMMA}/markets/slug/{slug}"))?;
    if !market.is_binary() {
        return Err(error(
            -3,
            format!("market '{slug}' is not a binary YES/NO market"),
        ));
    }
    if !market.active {
        return Err(error(-3, format!("market '{slug}' is not active")));
    }
    if market.closed {
        return Err(error(-3, format!("market '{slug}' is closed")));
    }
    if !market.enable_order_book {
        return Err(error(
            -3,
            format!("market '{slug}' does not have the order book enabled"),
        ));
    }
    let outcome = match outcome.to_ascii_uppercase().as_str() {
        "YES" => "YES",
        "NO" => "NO",
        other => return Err(error(-3, format!("outcome must be YES or NO, got {other}"))),
    };
    let token_id = match outcome {
        "YES" => market.yes_token_id(),
        "NO" => market.no_token_id(),
        _ => None,
    }
    .ok_or_else(|| error(-3, format!("market '{slug}' has no {outcome} token id")))?
    .to_string();
    let book: OrderBook = get_json(&url_with_query(
        &format!("{CLOB}/book"),
        &[("token_id", &token_id)],
    ))?;
    if !book.asset_id.is_empty() && book.asset_id != token_id {
        return Err(error(
            -4,
            format!(
                "CLOB book token mismatch: requested {token_id}, received {}",
                book.asset_id
            ),
        ));
    }
    if !book.market.is_empty()
        && !market.condition_id.is_empty()
        && book.market != market.condition_id
    {
        return Err(error(
            -4,
            format!(
                "CLOB book condition mismatch: Gamma {} vs CLOB {}",
                market.condition_id, book.market
            ),
        ));
    }
    if book.neg_risk != market.neg_risk {
        return Err(error(
            -4,
            format!(
                "neg_risk mismatch for '{slug}': Gamma={} CLOB={}",
                market.neg_risk, book.neg_risk
            ),
        ));
    }
    let tick_micro = if book.tick_size.trim().is_empty() {
        match market.order_price_min_tick_size {
            Some(tick) => parse_api_float_micro(tick, "orderPriceMinTickSize")?,
            None => return Err(error(-4, "CLOB book omitted tick_size")),
        }
    } else {
        parse_micro(&book.tick_size).map_err(|e| error(-4, e.to_string()))?
    };
    let min_size_micro = if book.min_order_size.trim().is_empty() {
        match market.order_min_size {
            Some(size) => parse_api_float_micro(size, "orderMinSize")?,
            None => 0,
        }
    } else {
        parse_micro(&book.min_order_size).map_err(|e| error(-4, e.to_string()))?
    };
    let best_ask_micro = best_price(&book.asks, true)?;
    let best_bid_micro = best_price(&book.bids, false)?;
    Ok(TradeSnapshot {
        market,
        outcome: outcome.into(),
        token_id,
        neg_risk: book.neg_risk,
        tick_micro,
        min_size_micro,
        best_ask_micro,
        best_bid_micro,
        active: true,
        closed: false,
        order_book_enabled: true,
    })
}

fn best_price(
    levels: &[bloom_polymarket::types::BookLevel],
    ask: bool,
) -> Result<Option<u64>, DispatchResponse> {
    let mut best: Option<u64> = None;
    for level in levels {
        let price = parse_micro(&level.price).map_err(|e| error(-4, e.to_string()))?;
        best = Some(match best {
            None => price,
            Some(existing) if ask => existing.min(price),
            Some(existing) => existing.max(price),
        });
    }
    Ok(best)
}

fn choose_trade_limit(
    side: Side,
    marketable: bool,
    bound_micro: u64,
    pinned_limit_micro: u64,
    snapshot: &TradeSnapshot,
) -> Result<u64, DispatchResponse> {
    shared_trade::choose_limit(
        side,
        marketable,
        bound_micro,
        pinned_limit_micro,
        &snapshot.as_shared(),
    )
    .map_err(polymarket_error)
}

fn build_trade_quote(
    side: Side,
    amount_micro: u64,
    limit_micro: u64,
    snapshot: &TradeSnapshot,
    order_type: OrderType,
) -> Result<LimitQuote, DispatchResponse> {
    shared_trade::build_quote(
        side,
        amount_micro,
        limit_micro,
        &snapshot.as_shared(),
        order_type,
    )
    .map_err(polymarket_error)
}

fn trade_policy_check(
    wallet: &str,
    draft: &StoreTradeDraft,
) -> Result<serde_json::Value, DispatchResponse> {
    let policy = wallet_policy(wallet)?;
    let (receipt_store_readable, daily_posted_microusd) = daily_posted_microusd(wallet);
    let ctx = LocalPolymarketOrderCtx {
        slug: draft.slug.clone(),
        condition_id: draft.condition_id.clone(),
        side: match draft.side {
            Side::Buy => LocalPolicySide::Buy,
            Side::Sell => LocalPolicySide::Sell,
        },
        amount_microusd: draft.amount_micro,
        limit_price_micro: draft.limit_price_micro,
        active: draft.active,
        closed: draft.closed,
        order_book_enabled: draft.order_book_enabled,
        binary_outcomes: draft.binary_outcomes,
        neg_risk: draft.neg_risk,
        receipt_store_readable,
        daily_posted_microusd,
    };
    let checks = evaluate_local_polymarket_order(&policy.polymarket, &ctx);
    let deny = local_policy_has_deny(&checks);
    let warn = local_policy_has_warn(&checks);
    let policy_status = if deny {
        "denied"
    } else if warn {
        "warn"
    } else {
        "passed"
    };
    let posting_enabled = !deny && draft.side == Side::Buy && draft.order_type != OrderType::GTD;
    let reason = if draft.side == Side::Sell {
        "sell posting requires passing authoritative chain CTF balance and approval checks"
    } else if draft.order_type == OrderType::GTD {
        "GTD posting is disabled until expiry parity is ported"
    } else {
        "buy can be posted after final review by writing to the post endpoint; resting GTC orders can be cancelled from their receipt"
    };
    Ok(serde_json::json!({
        "status": "blocked",
        "reason": reason,
        "policy_status": policy_status,
        "policy_deny": deny,
        "policy_warn": warn,
        "policy_checks": checks,
        "receipt_store_readable": receipt_store_readable,
        "daily_posted_microusd": daily_posted_microusd,
        "receipt_audit_parity": true,
        "active": draft.active,
        "closed": draft.closed,
        "binary_outcomes": draft.binary_outcomes,
        "order_book_enabled": draft.order_book_enabled,
        "size_at_or_above_min": draft.size_micro >= draft.min_order_size_micro,
        "signing_enabled": posting_enabled,
        "posting_enabled": posting_enabled
    }))
}

fn enable_trade_posting(policy_check: &mut serde_json::Value, reason: &str) {
    if let Some(obj) = policy_check.as_object_mut() {
        obj.insert("status".into(), serde_json::Value::String("ready".into()));
        obj.insert("reason".into(), serde_json::Value::String(reason.into()));
        obj.insert("signing_enabled".into(), serde_json::Value::Bool(true));
        obj.insert("posting_enabled".into(), serde_json::Value::Bool(true));
    }
}

fn wallet_policy(wallet: &str) -> Result<LocalWalletPolicy, DispatchResponse> {
    let bytes =
        bloom_petal_sdk::vfs_read(&format!("wallets/{wallet}/policy.toml"), MAX_POLICY_BYTES)
            .map_err(sdk_error)?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("wallet policy is not utf-8: {e}")))?;
    toml::from_str(raw).map_err(|e| error(-4, format!("wallet policy parse: {e}")))
}

fn daily_posted_microusd(wallet: &str) -> (bool, Option<u64>) {
    let prefix = format!("trade/{wallet}/receipts/");
    let keys = match bloom_petal_sdk::store_list(&prefix, MAX_LIST_BYTES) {
        Ok(keys) => keys,
        Err(_) => return (false, None),
    };
    let cutoff = now_millis().saturating_sub(24 * 60 * 60 * 1000);
    let mut present = BTreeSet::new();
    for key in &keys {
        let rest = key.strip_prefix(&prefix).unwrap_or(key);
        let Some(id) = rest.strip_suffix("/receipt.json") else {
            continue;
        };
        present.insert(id.to_string());
    }
    let audited = match audited_receipt_ids_since(wallet, cutoff) {
        Ok(ids) => ids,
        Err(_) => return (false, None),
    };
    for id in audited {
        if !present.contains(&id) {
            return (false, None);
        }
    }
    let mut total = 0u64;
    for key in keys {
        if !key.ends_with("/receipt.json") {
            continue;
        }
        let Some(bytes) = store_get(&key) else {
            return (false, None);
        };
        let receipt: StoreTradeReceiptPolicy = match serde_json::from_slice(&bytes) {
            Ok(receipt) => receipt,
            Err(_) => return (false, None),
        };
        if receipt.posted_ms < cutoff || receipt.side != Side::Buy {
            continue;
        }
        if clob_status_excluded_from_daily_cap(receipt.clob_status.as_str(), receipt.order_type) {
            continue;
        }
        total = total.saturating_add(receipt.amount_microusd);
    }
    (true, Some(total))
}

fn audited_receipt_ids_since(wallet: &str, cutoff_ms: u128) -> Result<Vec<String>, SdkError> {
    let key = format!("trade/{wallet}/audit.jsonl");
    let bytes = match bloom_petal_sdk::store_get(&key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = core::str::from_utf8(&bytes).map_err(|_| SdkError::Host(HostStatus::Invalid))?;
    let mut ids = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("event").and_then(serde_json::Value::as_str) != Some("receipt_written") {
            continue;
        }
        let ts = v
            .get("ts_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u128;
        if ts < cutoff_ms {
            continue;
        }
        if let Some(id) = v
            .get("details")
            .and_then(|details| details.get("draft_id"))
            .and_then(serde_json::Value::as_str)
        {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn verify_sell_preflight(
    wallet: &str,
    owner: Address,
    deposit: Address,
    token_id: &str,
    size_micro: u64,
    neg_risk: bool,
) -> Result<serde_json::Value, DispatchResponse> {
    let deposit_user = deposit.to_checksum(None);
    let data_api_holding_micro = get_json::<Vec<Position>>(&url_with_query(
        &format!("{DATA}/positions"),
        &[("user", &deposit_user)],
    ))
    .ok()
    .map(|positions| {
        positions
            .iter()
            .find(|position| position.asset == token_id)
            .and_then(position_size_micro)
            .unwrap_or(0)
    });

    let creds = load_creds(wallet)?;
    let clob_balance_allowance = clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[
            ("asset_type", "CONDITIONAL"),
            ("token_id", token_id),
            ("signature_type", "3"),
        ],
    )?;
    let clob_balance_micro = clob_balance_allowance
        .get("balance")
        .and_then(parse_clob_raw_micro)
        .ok_or_else(|| error(-4, "CLOB conditional balance response missing balance"))?;
    if clob_balance_micro < size_micro {
        return Err(error(
            -3,
            format!(
                "cannot sell {} shares: CLOB conditional balance reports only {}",
                format_micro(size_micro),
                format_micro(clob_balance_micro)
            ),
        ));
    }
    let operator = if neg_risk {
        NEG_RISK_EXCHANGE_V2
    } else {
        CTF_EXCHANGE_V2
    };
    let chain_ctf_balance = read_chain_ctf_balance(deposit, token_id)?;
    if chain_ctf_balance < size_micro {
        return Err(error(
            -3,
            format!(
                "cannot sell {} shares: on-chain CTF balance for derived deposit wallet {} is only {}",
                format_micro(size_micro),
                deposit.to_checksum(None),
                format_micro(chain_ctf_balance)
            ),
        ));
    }
    let ctf_approved = read_chain_ctf_approval(deposit, operator)?;
    if !ctf_approved {
        return Err(error(
            -3,
            format!(
                "cannot sell before passkey: deposit wallet {} has not approved {} for CTF tokens. Re-run onboarding to restore approvals.",
                deposit.to_checksum(None),
                operator.to_checksum(None)
            ),
        ));
    }

    Ok(serde_json::json!({
        "status": "pass",
        "source": "clob_conditional_balance_and_chain_ctf",
        "preflight_complete_for_posting": true,
        "chain_ctf_balance_checked": true,
        "ctf_approval_checked": true,
        "reason": "sell preflight passed CLOB conditional balance, on-chain CTF balance, and CTF operator approval checks; Data API holdings are included as corroborating evidence when available",
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": "live_factory_resolved",
        "token_id": token_id,
        "requested_size_micro": size_micro,
        "requested_size": format_micro(size_micro),
        "data_api_holding_checked": data_api_holding_micro.is_some(),
        "data_api_holding_micro": data_api_holding_micro,
        "data_api_holding": data_api_holding_micro.map(format_micro),
        "clob_balance_micro": clob_balance_micro,
        "clob_balance": format_micro(clob_balance_micro),
        "clob_balance_allowance": clob_balance_allowance,
        "chain_ctf_contract": CTF.to_checksum(None),
        "chain_ctf_balance_micro": chain_ctf_balance,
        "chain_ctf_balance": format_micro(chain_ctf_balance),
        "ctf_operator": operator.to_checksum(None),
        "ctf_operator_kind": if neg_risk { "neg_risk_exchange_v2" } else { "ctf_exchange_v2" },
        "ctf_approved_for_all": ctf_approved,
        "signing_enabled": true,
        "posting_enabled": true
    }))
}

fn read_chain_ctf_balance(deposit: Address, token_id: &str) -> Result<u64, DispatchResponse> {
    let response = read_chain_method(
        CTF,
        "balanceOf",
        &serde_json::json!({
            "args": [deposit.to_checksum(None), token_id]
        }),
    )?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, "chain CTF balanceOf response missing decoded array"))?;
    let raw = decoded
        .first()
        .ok_or_else(|| error(-4, "chain CTF balanceOf response missing balance"))?;
    parse_clob_raw_micro(raw).ok_or_else(|| error(-4, "chain CTF balance is not a u64"))
}

fn read_chain_ctf_approval(deposit: Address, operator: Address) -> Result<bool, DispatchResponse> {
    let response = read_chain_method(
        CTF,
        "isApprovedForAll",
        &serde_json::json!({
            "args": [deposit.to_checksum(None), operator.to_checksum(None)]
        }),
    )?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            error(
                -4,
                "chain CTF isApprovedForAll response missing decoded array",
            )
        })?;
    decoded
        .first()
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| error(-4, "chain CTF approval response is not a boolean"))
}

fn read_chain_deposit_wallet_deployed(address: Address) -> Result<bool, DispatchResponse> {
    let path = format!(
        "chains/polygon/contracts/{}/proxy/implementation",
        address.to_checksum(None)
    );
    let bytes = bloom_petal_sdk::vfs_read(&path, MAX_CHAIN_READ_BYTES)
        .map_err(|e| sdk_error_with_context("read deposit wallet proxy implementation", e))?;
    let text = core::str::from_utf8(&bytes)
        .map_err(|_| error(-4, "chain proxy implementation response is not UTF-8"))?;
    let text = text.trim();
    if text == "not a proxy" {
        return Ok(false);
    }
    text.parse::<Address>().map(|_| true).map_err(|e| {
        error(
            -4,
            format!("chain proxy implementation response is not an address: {e}"),
        )
    })
}

fn read_chain_erc20_balance(token: Address, holder: Address) -> Result<U256, DispatchResponse> {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    let mut encoded_holder = [0u8; 32];
    encoded_holder[12..].copy_from_slice(holder.as_slice());
    calldata.extend_from_slice(&encoded_holder);
    read_chain_eth_call_u256(token, &calldata, "chain ERC20 balanceOf")
}

fn read_chain_erc20_allowance(
    token: Address,
    owner: Address,
    spender: Address,
) -> Result<U256, DispatchResponse> {
    let mut calldata = Vec::with_capacity(68);
    calldata.extend_from_slice(&[0xdd, 0x62, 0xed, 0x3e]);
    for address in [owner, spender] {
        let mut encoded = [0u8; 32];
        encoded[12..].copy_from_slice(address.as_slice());
        calldata.extend_from_slice(&encoded);
    }
    read_chain_eth_call_u256(token, &calldata, "chain ERC20 allowance")
}

fn read_chain_v2_approvals(deposit: Address) -> Result<bool, DispatchResponse> {
    let floor = allowance_floor();
    for spender in v2_spenders() {
        if read_chain_erc20_allowance(PUSD, deposit, spender)? < floor {
            return Ok(false);
        }
    }
    for operator in v2_spenders() {
        if !read_chain_ctf_approval(deposit, operator)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_chain_v2_approvals_revoked(deposit: Address) -> Result<bool, DispatchResponse> {
    for spender in v2_spenders() {
        if !read_chain_erc20_allowance(PUSD, deposit, spender)?.is_zero() {
            return Ok(false);
        }
    }
    for operator in v2_spenders() {
        if read_chain_ctf_approval(deposit, operator)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_clob_collateral_sync(
    owner: Address,
    creds: &Credentials,
) -> Result<(bool, Option<U256>, Option<U256>), DispatchResponse> {
    let value = clob_l2_get_json(
        owner,
        creds,
        "/balance-allowance",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    )?;
    let balance = value.get("balance").and_then(parse_json_u256);
    let allowance = value.get("allowance").and_then(parse_json_u256);
    Ok((
        balance.map(|v| !v.is_zero()).unwrap_or(false)
            && allowance.map(|v| !v.is_zero()).unwrap_or(false),
        balance,
        allowance,
    ))
}

fn parse_json_u256(value: &serde_json::Value) -> Option<U256> {
    match value {
        serde_json::Value::String(s) => s.trim().parse::<U256>().ok(),
        serde_json::Value::Number(n) => n.as_u64().map(U256::from),
        _ => None,
    }
}

fn allowance_floor() -> U256 {
    U256::from(1) << 160
}

fn v2_spenders() -> [Address; 4] {
    [
        CTF_EXCHANGE_V2,
        NEG_RISK_EXCHANGE_V2,
        CTF_COLLATERAL_ADAPTER,
        NEG_RISK_CTF_COLLATERAL_ADAPTER,
    ]
}

fn predict_deposit_wallet(owner: Address) -> Result<Address, DispatchResponse> {
    let implementation = read_chain_address(
        FACTORY,
        "implementation",
        &serde_json::json!({ "args": [] }),
        "factory implementation",
    )?;
    let wallet_id = format!("0x{}{}", "00".repeat(12), hex::encode(owner.as_slice()));
    read_chain_address(
        FACTORY,
        "predictWalletAddress",
        &serde_json::json!({
            "args": [implementation.to_checksum(None), wallet_id]
        }),
        "factory predictWalletAddress",
    )
}

fn read_chain_address(
    contract: Address,
    method: &str,
    body: &serde_json::Value,
    label: &str,
) -> Result<Address, DispatchResponse> {
    let response = read_chain_method(contract, method, body)?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, format!("{label} response missing decoded array")))?;
    decoded
        .first()
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, format!("{label} response is not an address")))?
        .parse::<Address>()
        .map_err(|e| error(-4, format!("{label} address parse: {e}")))
}

fn read_chain_method(
    contract: Address,
    method: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, DispatchResponse> {
    let nonce = chain_method_nonce();
    let path = format!(
        "chains/polygon/contracts/{}/methods/{method}@{nonce}.read",
        contract.to_checksum(None)
    );
    let bytes =
        serde_json::to_vec(body).map_err(|e| error(-4, format!("chain method body: {e}")))?;
    bloom_petal_sdk::vfs_write(&path, &bytes)
        .map_err(|e| sdk_error_with_context("stage chain method read", e))?;
    let response = bloom_petal_sdk::vfs_read(&path, MAX_CHAIN_METHOD_BYTES)
        .map_err(|e| sdk_error_with_context("read chain method result", e))?;
    serde_json::from_slice(&response).map_err(|e| error(-4, format!("chain method JSON: {e}")))
}

fn chain_method_nonce() -> String {
    let bytes = bloom_petal_sdk::random_bytes(16).unwrap_or_else(|_| {
        let mut fallback = [0u8; 16];
        fallback[..8].copy_from_slice(&now_millis().to_be_bytes());
        fallback.to_vec()
    });
    hex::encode(bytes)
}

fn position_size_micro(position: &Position) -> Option<u64> {
    position
        .size
        .and_then(|size| parse_json_f64_micro(size).ok())
}

fn parse_clob_raw_micro(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::String(s) => s.trim().parse::<u64>().ok(),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Some(u)
            } else {
                n.as_f64().and_then(|f| parse_json_f64_micro(f).ok())
            }
        }
        _ => None,
    }
}

fn parse_json_f64_micro(value: f64) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, "decimal value is not a non-negative number"));
    }
    parse_micro(&format!("{value}")).map_err(|e| error(-4, e.to_string()))
}

fn evaluate_local_polymarket_order(
    policy: &LocalPolymarketPolicy,
    ctx: &LocalPolymarketOrderCtx,
) -> Vec<LocalPolicyCheck> {
    let mut out = Vec::new();
    if !policy.enabled {
        out.push(local_policy_check(
            "enabled",
            LocalPolicyOutcome::Deny,
            "Polymarket trading is disabled for this wallet; set [polymarket] enabled = true in the wallet policy to opt in",
        ));
    } else {
        out.push(local_policy_check(
            "enabled",
            LocalPolicyOutcome::Pass,
            "trading enabled",
        ));
    }

    if ctx.closed || !ctx.active {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            format!(
                "market is not tradable (active={}, closed={})",
                ctx.active, ctx.closed
            ),
        ));
    } else if !ctx.order_book_enabled {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            "market has no order book enabled",
        ));
    } else if !ctx.binary_outcomes {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            "market is malformed or not a binary YES/NO market",
        ));
    } else {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Pass,
            "active, order book enabled, binary outcomes",
        ));
    }

    out.push(local_policy_list_check(
        "slug",
        &ctx.slug,
        &policy.allowed_slugs,
        &policy.denied_slugs,
    ));
    out.push(local_policy_list_check(
        "condition_id",
        &ctx.condition_id,
        &policy.allowed_condition_ids,
        &policy.denied_condition_ids,
    ));

    if ctx.neg_risk && !policy.allow_neg_risk {
        out.push(local_policy_check(
            "neg_risk",
            LocalPolicyOutcome::Deny,
            "neg-risk markets are disabled by policy (allow_neg_risk = false)",
        ));
    } else {
        out.push(local_policy_check(
            "neg_risk",
            LocalPolicyOutcome::Pass,
            format!("neg_risk={} permitted", ctx.neg_risk),
        ));
    }

    if ctx.side == LocalPolicySide::Sell {
        out.push(local_policy_check(
            "caps",
            LocalPolicyOutcome::Pass,
            "sell orders are risk-reducing; USD caps not applied",
        ));
        return out;
    }

    if let Some(cap) = policy.max_order_usd {
        if ctx.amount_microusd > cap {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Deny,
                format!(
                    "order {} USD exceeds max_order_usd {}",
                    format_micro(ctx.amount_microusd),
                    format_micro(cap)
                ),
            ));
        } else {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Pass,
                format!(
                    "{} <= {}",
                    format_micro(ctx.amount_microusd),
                    format_micro(cap)
                ),
            ));
        }
    }

    if let Some(cap) = policy.max_daily_usd {
        match (ctx.receipt_store_readable, ctx.daily_posted_microusd) {
            (false, _) | (_, None) => out.push(local_policy_check(
                "max_daily_usd",
                LocalPolicyOutcome::Deny,
                "daily cap configured but posted exposure is unknown (receipt store unreadable) - refusing rather than trading uncapped",
            )),
            (true, Some(daily)) => {
                let total = daily.saturating_add(ctx.amount_microusd);
                if total > cap {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Deny,
                        format!(
                            "posted {} USD + order {} USD exceeds max_daily_usd {}",
                            format_micro(daily),
                            format_micro(ctx.amount_microusd),
                            format_micro(cap)
                        ),
                    ));
                } else {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Pass,
                        format!(
                            "{} + {} <= {}",
                            format_micro(daily),
                            format_micro(ctx.amount_microusd),
                            format_micro(cap)
                        ),
                    ));
                }
            }
        }
    }

    if let Some(maxp) = policy.max_price {
        if ctx.limit_price_micro > maxp {
            out.push(local_policy_check(
                "max_price",
                LocalPolicyOutcome::Deny,
                format!(
                    "limit price {} exceeds policy max_price {}",
                    format_micro(ctx.limit_price_micro),
                    format_micro(maxp)
                ),
            ));
        } else {
            out.push(local_policy_check(
                "max_price",
                LocalPolicyOutcome::Pass,
                format!(
                    "{} <= {}",
                    format_micro(ctx.limit_price_micro),
                    format_micro(maxp)
                ),
            ));
        }
    }

    if let Some(threshold) = policy.require_flag_above_usd
        && ctx.amount_microusd > threshold
    {
        out.push(local_policy_check(
            "require_flag_above_usd",
            LocalPolicyOutcome::Warn,
            format!(
                "order {} USD is above {} - acknowledge before value-moving post",
                format_micro(ctx.amount_microusd),
                format_micro(threshold)
            ),
        ));
    }

    out
}

fn local_policy_check(
    rule: &str,
    outcome: LocalPolicyOutcome,
    message: impl Into<String>,
) -> LocalPolicyCheck {
    LocalPolicyCheck {
        rule: format!("polymarket.{rule}"),
        outcome,
        message: message.into(),
    }
}

fn local_policy_list_check(
    name: &str,
    value: &str,
    allowed: &BTreeSet<String>,
    denied: &BTreeSet<String>,
) -> LocalPolicyCheck {
    if denied.contains(value) {
        local_policy_check(
            name,
            LocalPolicyOutcome::Deny,
            format!("'{value}' is denylisted"),
        )
    } else if !allowed.is_empty() && !allowed.contains(value) {
        local_policy_check(
            name,
            LocalPolicyOutcome::Deny,
            format!("'{value}' is not on the allowlist (allowlist-only mode)"),
        )
    } else {
        local_policy_check(
            name,
            LocalPolicyOutcome::Pass,
            format!("'{value}' permitted"),
        )
    }
}

fn local_policy_has_deny(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Deny)
}

fn local_policy_has_warn(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Warn)
}

fn parse_api_float_micro(value: f64, field: &str) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, format!("{field} is not a non-negative number")));
    }
    parse_micro(&format!("{value:.6}")).map_err(|e| error(-4, e.to_string()))
}

fn read_trade(wallet: &str, kind: &str, id: &str, file: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    match (kind, file) {
        ("drafts", "plan.md") => {
            let Some(bytes) = store_get(&format!("trade/{wallet}/drafts/{id}/order.json")) else {
                return error(-1, "not found");
            };
            let draft: StoreTradeDraft = match serde_json::from_slice(&bytes) {
                Ok(draft) => draft,
                Err(e) => return error(-4, format!("corrupt draft: {e}")),
            };
            DispatchResponse::Read(render_trade_plan(&draft).into_bytes())
        }
        (
            "drafts",
            "order.json" | "policy_check.json" | "quote.json" | "review_intent.json"
            | "post_attempt.json" | "approval.json",
        ) => read_store(&format!("trade/{wallet}/drafts/{id}/{file}")),
        ("drafts", "revalidate") => DispatchResponse::Read(TRADE_REVALIDATE_HINT.into()),
        ("drafts", "post") => DispatchResponse::Read(TRADE_POST_HINT.into()),
        ("receipts", "receipt.json") => {
            read_store(&format!("trade/{wallet}/receipts/{id}/receipt.json"))
        }
        ("receipts", "cancel") => DispatchResponse::Read(TRADE_CANCEL_HINT.into()),
        _ => error(-3, "not a trade file"),
    }
}

fn load_trade_draft(
    wallet: &str,
    id: &str,
    base: &str,
) -> Result<StoreTradeDraft, DispatchResponse> {
    let Some(bytes) = store_get(&format!("{base}/order.json")) else {
        return Err(error(-1, "draft not found"));
    };
    let draft: StoreTradeDraft =
        serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt draft: {e}")))?;
    if draft.wallet != wallet || draft.id != id {
        return Err(error(-4, "draft identity mismatch"));
    }
    Ok(draft)
}

fn write_trade_revalidate(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid draft id");
    }
    let req: TradeRevalidateRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("revalidate JSON: {e}")),
    };
    if !req.revalidate {
        return error(-3, "revalidate must be true");
    }
    let base = format!("trade/{wallet}/drafts/{id}");
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let mut draft = match load_trade_draft(wallet, id, &base) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if draft.status != "review" && draft.status != "revalidated" {
        return error(
            -3,
            format!("draft {id} is '{}' and cannot be revalidated", draft.status),
        );
    }
    if draft.order_type == OrderType::GTD {
        return error(-3, "posting GTD orders is pending expiry parity");
    }

    let snapshot = match trade_snapshot(&draft.slug, &draft.outcome) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    if snapshot.token_id != draft.token_id {
        return error(
            -3,
            "token id changed between draft and revalidate; refusing",
        );
    }
    if snapshot.market.condition_id != draft.condition_id {
        return error(
            -3,
            "condition id changed between draft and revalidate; refusing",
        );
    }
    if snapshot.neg_risk != draft.neg_risk {
        return error(
            -3,
            "neg-risk changed between draft and revalidate; refusing",
        );
    }
    let amount_input = match draft.side {
        Side::Buy => draft.amount_micro.max(1),
        Side::Sell => draft.size_micro,
    };
    let limit_micro = match choose_trade_limit(
        draft.side,
        draft.marketable,
        draft.price_bound_micro,
        draft.limit_price_micro,
        &snapshot,
    ) {
        Ok(limit) => limit,
        Err(resp) => return resp,
    };
    let quote = match build_trade_quote(
        draft.side,
        amount_input,
        limit_micro,
        &snapshot,
        draft.order_type,
    ) {
        Ok(quote) => quote,
        Err(resp) => return resp,
    };

    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let funder = match tradeable_deposit_wallet(wallet, owner) {
        Ok(funder) => funder,
        Err(resp) => return resp,
    };

    draft.limit_price_micro = quote.price_micro;
    draft.size_micro = quote.size_micro;
    draft.maker_micro = quote.maker_micro;
    draft.taker_micro = quote.taker_micro;
    if draft.side == Side::Sell {
        draft.amount_micro = draft.taker_micro;
    }
    draft.tick_micro = snapshot.tick_micro;
    draft.min_order_size_micro = snapshot.min_size_micro;
    draft.neg_risk = snapshot.neg_risk;
    draft.active = snapshot.active;
    draft.closed = snapshot.closed;
    draft.order_book_enabled = snapshot.order_book_enabled;
    draft.binary_outcomes = snapshot.market.is_binary();
    draft.best_ask_micro = snapshot.best_ask_micro;
    draft.best_bid_micro = snapshot.best_bid_micro;
    draft.book_snapshot_secs = now_secs();
    draft.status = "revalidated".into();
    let mut policy_check = match trade_policy_check(wallet, &draft) {
        Ok(check) => check,
        Err(resp) => return resp,
    };
    let policy_deny = policy_check
        .get("policy_deny")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let policy_status = policy_check
        .get("policy_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/quote.json"),
        &serde_json::json!({
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "marketable": draft.marketable,
            "amount_micro": draft.amount_micro,
            "amount": format_micro(draft.amount_micro),
            "price_bound_micro": draft.price_bound_micro,
            "price_bound": format_micro(draft.price_bound_micro),
            "limit_price_micro": draft.limit_price_micro,
            "limit_price": format_micro(draft.limit_price_micro),
            "size_micro": draft.size_micro,
            "size": format_micro(draft.size_micro),
            "maker_micro": draft.maker_micro,
            "maker": format_micro(draft.maker_micro),
            "taker_micro": draft.taker_micro,
            "taker": format_micro(draft.taker_micro),
            "tick_micro": draft.tick_micro,
            "tick": format_micro(draft.tick_micro),
            "min_order_size_micro": draft.min_order_size_micro,
            "min_order_size": format_micro(draft.min_order_size_micro),
            "best_ask_micro": draft.best_ask_micro,
            "best_bid_micro": draft.best_bid_micro,
            "status": "revalidated"
        }),
        false,
    ) {
        return error(-4, "failed to store quote");
    }
    if policy_deny {
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
        {
            return error(-4, "failed to store policy check");
        }
        match bloom_petal_sdk::store_del(&format!("{base}/review_intent.json")) {
            Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
            Err(_) => return error(-4, "failed to clear stale review intent"),
        }
        draft.status = "policy_denied".into();
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/order.json"), &draft, false)
        {
            return error(-4, "failed to store denied draft");
        }
        return error(-3, "Polymarket policy denied; see policy_check.json");
    }
    let sell_preflight = if draft.side == Side::Sell {
        match verify_sell_preflight(
            wallet,
            owner,
            funder,
            &draft.token_id,
            draft.size_micro,
            draft.neg_risk,
        ) {
            Ok(preflight) => {
                enable_trade_posting(
                    &mut policy_check,
                    "sell can be posted after final review because chain CTF balance and approval checks passed",
                );
                Some(preflight)
            }
            Err(resp) => {
                match bloom_petal_sdk::store_del(&format!("{base}/review_intent.json")) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
                    Err(_) => return error(-4, "failed to clear stale review intent"),
                }
                draft.status = "preflight_denied".into();
                if let DispatchResponse::Error { .. } =
                    store_put_json(&format!("{base}/order.json"), &draft, false)
                {
                    return error(-4, "failed to store denied draft");
                }
                return resp;
            }
        }
    } else {
        None
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return error(-4, "failed to store policy check");
    }
    let posting_enabled = policy_check
        .get("posting_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/review_intent.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "slug": draft.slug,
            "condition_id": draft.condition_id,
            "outcome": draft.outcome,
            "token_id": draft.token_id,
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "limit_price": format_micro(draft.limit_price_micro),
            "size": format_micro(draft.size_micro),
            "maker": format_micro(draft.maker_micro),
            "taker": format_micro(draft.taker_micro),
            "neg_risk": draft.neg_risk,
            "policy_status": policy_status,
            "sell_preflight": sell_preflight,
            "status": "final_review_staged",
            "signing_enabled": posting_enabled,
            "posting_enabled": posting_enabled
        }),
        false,
    ) {
        return error(-4, "failed to store review intent");
    }
    store_put_json(&format!("{base}/order.json"), &draft, false)
}

fn refresh_trade_post_inputs(
    wallet: &str,
    base: &str,
    draft: &mut StoreTradeDraft,
    owner: Address,
) -> Result<(serde_json::Value, Option<serde_json::Value>), DispatchResponse> {
    let snapshot = trade_snapshot(&draft.slug, &draft.outcome)?;
    if snapshot.token_id != draft.token_id {
        return Err(error(
            -3,
            "token id changed between draft and post; refusing",
        ));
    }
    if snapshot.market.condition_id != draft.condition_id {
        return Err(error(
            -3,
            "condition id changed between draft and post; refusing",
        ));
    }
    if snapshot.neg_risk != draft.neg_risk {
        return Err(error(
            -3,
            "neg-risk changed between draft and post; refusing",
        ));
    }
    let amount_input = match draft.side {
        Side::Buy => draft.amount_micro.max(1),
        Side::Sell => draft.size_micro,
    };
    let limit_micro = choose_trade_limit(
        draft.side,
        draft.marketable,
        draft.price_bound_micro,
        draft.limit_price_micro,
        &snapshot,
    )?;
    let quote = build_trade_quote(
        draft.side,
        amount_input,
        limit_micro,
        &snapshot,
        draft.order_type,
    )?;
    draft.limit_price_micro = quote.price_micro;
    draft.size_micro = quote.size_micro;
    draft.maker_micro = quote.maker_micro;
    draft.taker_micro = quote.taker_micro;
    if draft.side == Side::Sell {
        draft.amount_micro = draft.taker_micro;
    }
    draft.tick_micro = snapshot.tick_micro;
    draft.min_order_size_micro = snapshot.min_size_micro;
    draft.neg_risk = snapshot.neg_risk;
    draft.active = snapshot.active;
    draft.closed = snapshot.closed;
    draft.order_book_enabled = snapshot.order_book_enabled;
    draft.binary_outcomes = snapshot.market.is_binary();
    draft.best_ask_micro = snapshot.best_ask_micro;
    draft.best_bid_micro = snapshot.best_bid_micro;
    draft.book_snapshot_secs = now_secs();
    let mut policy_check = trade_policy_check(wallet, draft)?;
    let policy_deny = policy_check
        .get("policy_deny")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/quote.json"),
        &serde_json::json!({
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "marketable": draft.marketable,
            "amount_micro": draft.amount_micro,
            "amount": format_micro(draft.amount_micro),
            "price_bound_micro": draft.price_bound_micro,
            "price_bound": format_micro(draft.price_bound_micro),
            "limit_price_micro": draft.limit_price_micro,
            "limit_price": format_micro(draft.limit_price_micro),
            "size_micro": draft.size_micro,
            "size": format_micro(draft.size_micro),
            "maker_micro": draft.maker_micro,
            "maker": format_micro(draft.maker_micro),
            "taker_micro": draft.taker_micro,
            "taker": format_micro(draft.taker_micro),
            "tick_micro": draft.tick_micro,
            "tick": format_micro(draft.tick_micro),
            "min_order_size_micro": draft.min_order_size_micro,
            "min_order_size": format_micro(draft.min_order_size_micro),
            "best_ask_micro": draft.best_ask_micro,
            "best_bid_micro": draft.best_bid_micro,
            "status": "post_revalidated"
        }),
        false,
    ) {
        return Err(error(-4, "failed to store quote"));
    }
    if policy_deny {
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
        {
            return Err(error(-4, "failed to store policy check"));
        }
        match bloom_petal_sdk::store_del(&format!("{base}/review_intent.json")) {
            Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
            Err(_) => return Err(error(-4, "failed to clear stale review intent")),
        }
        draft.status = "policy_denied".into();
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/order.json"), draft, false)
        {
            return Err(error(-4, "failed to store denied draft"));
        }
        return Err(error(-3, "Polymarket policy denied; see policy_check.json"));
    }
    let sell_preflight = if draft.side == Side::Sell {
        let funder = tradeable_deposit_wallet(wallet, owner)?;
        let preflight = verify_sell_preflight(
            wallet,
            owner,
            funder,
            &draft.token_id,
            draft.size_micro,
            draft.neg_risk,
        )?;
        enable_trade_posting(
            &mut policy_check,
            "sell can be posted after final review because chain CTF balance and approval checks passed",
        );
        Some(preflight)
    } else {
        None
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return Err(error(-4, "failed to store policy check"));
    }
    Ok((policy_check, sell_preflight))
}

fn review_intent_matches_draft(
    review: &serde_json::Value,
    draft: &StoreTradeDraft,
    owner: Address,
    funder: Address,
    policy_check: &serde_json::Value,
    sell_preflight: Option<&serde_json::Value>,
) -> Result<(), String> {
    let side = match draft.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let policy_status = policy_check
        .get("policy_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    for (field, expected) in [
        ("wallet", draft.wallet.clone()),
        ("draft_id", draft.id.clone()),
        ("owner", owner.to_checksum(None)),
        ("funder", funder.to_checksum(None)),
        ("slug", draft.slug.clone()),
        ("condition_id", draft.condition_id.clone()),
        ("outcome", draft.outcome.clone()),
        ("token_id", draft.token_id.clone()),
        ("side", side.to_string()),
        ("order_type", draft.order_type.as_str().to_string()),
        ("limit_price", format_micro(draft.limit_price_micro)),
        ("size", format_micro(draft.size_micro)),
        ("maker", format_micro(draft.maker_micro)),
        ("taker", format_micro(draft.taker_micro)),
        ("policy_status", policy_status.to_string()),
    ] {
        if review.get(field).and_then(serde_json::Value::as_str) != Some(expected.as_str()) {
            return Err(format!(
                "final review field '{field}' no longer matches live post inputs"
            ));
        }
    }
    if review.get("neg_risk").and_then(serde_json::Value::as_bool) != Some(draft.neg_risk) {
        return Err("final review field 'neg_risk' no longer matches live post inputs".into());
    }
    if draft.side == Side::Sell {
        let Some(fresh) = sell_preflight else {
            return Err("final review field 'sell_preflight' is missing live post evidence".into());
        };
        if review.get("sell_preflight") != Some(fresh) {
            return Err(
                "final review field 'sell_preflight' no longer matches live post inputs".into(),
            );
        }
    }
    Ok(())
}

fn write_trade_post(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid draft id");
    }
    let req: TradePostRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("post JSON: {e}")),
    };
    if !req.post {
        return error(-3, "post must be true");
    }
    let base = format!("trade/{wallet}/drafts/{id}");
    let policy_check =
        match bloom_petal_sdk::store_get(&format!("{base}/policy_check.json"), MAX_STORE_BYTES) {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(policy) => policy,
                Err(e) => return error(-4, format!("corrupt policy check: {e}")),
            },
            Err(e) => return sdk_error(e),
        };
    if !trade_post_policy_acknowledged(&policy_check, req.confirm_risk) {
        return error(-3, "policy warnings require confirm_risk=true");
    }
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let mut draft = match load_trade_draft(wallet, id, &base) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if draft.status != "revalidated" && draft.status != "signing_prepared" {
        return error(
            -3,
            format!(
                "draft {id} is '{}' and cannot be posted; write revalidate first",
                draft.status
            ),
        );
    }
    if draft.order_type == OrderType::GTD {
        return error(-3, "posting GTD orders is pending expiry parity");
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let prepared_key = format!("{base}/prepared_order.json");
    let approval_key = format!("{base}/approval.json");
    let (order, funder, review_intent_hash, prepared) = if draft.status == "signing_prepared" {
        let Some(PreparedSigning::Order(prepared_order)) =
            (match load_prepared_signing(&prepared_key) {
                Ok(prepared) => prepared,
                Err(response) => return response,
            })
        else {
            return error(-4, "missing prepared order for signing retry");
        };
        if prepared_order.draft_id != id
            || prepared_order.owner != owner.to_checksum(None)
            || prepared_order.condition_id != draft.condition_id
            || prepared_order.token_id != draft.token_id
            || prepared_order.side != draft.side as u8
            || prepared_order.price_micro != draft.limit_price_micro
            || prepared_order.size_micro != draft.size_micro
            || prepared_order.maker_amount != draft.maker_micro.to_string()
            || prepared_order.taker_amount != draft.taker_micro.to_string()
            || prepared_order.order_type != draft.order_type.as_str()
            || prepared_order.neg_risk != draft.neg_risk
            || prepared_order.chain_id != POLYGON
        {
            return error(-4, "prepared order does not match this draft");
        }
        let funder = match prepared_order.funder.parse::<Address>() {
            Ok(funder) => funder,
            Err(e) => return error(-4, format!("corrupt prepared order funder: {e}")),
        };
        let order = match prepared_order.order() {
            Ok(order) => order,
            Err(response) => return response,
        };
        let prepared = PreparedSigning::Order(prepared_order);
        if prepared.signing_hash().ok() != Some(poly1271_digest(&order, POLYGON, draft.neg_risk)) {
            return error(-4, "prepared order hash does not match its preimage");
        }
        (
            order,
            funder,
            match &prepared {
                PreparedSigning::Order(order) => order.review_intent_hash.clone(),
                _ => unreachable!(),
            },
            prepared,
        )
    } else {
        let funder = match tradeable_deposit_wallet(wallet, owner) {
            Ok(funder) => funder,
            Err(resp) => return resp,
        };
        let (policy_check, sell_preflight) =
            match refresh_trade_post_inputs(wallet, &base, &mut draft, owner) {
                Ok(inputs) => inputs,
                Err(resp) => return resp,
            };
        let review_intent_bytes = match bloom_petal_sdk::store_get(
            &format!("{base}/review_intent.json"),
            MAX_STORE_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(SdkError::Host(HostStatus::NotFound)) => {
                return error(-3, "missing final review intent; write revalidate first");
            }
            Err(e) => return sdk_error(e),
        };
        let review_intent: serde_json::Value = match serde_json::from_slice(&review_intent_bytes) {
            Ok(value) => value,
            Err(e) => return error(-4, format!("corrupt review intent: {e}")),
        };
        if review_intent
            .get("status")
            .and_then(serde_json::Value::as_str)
            != Some("final_review_staged")
            || review_intent
                .get("posting_enabled")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
        {
            return error(-3, "final review intent does not enable posting");
        }
        if let Err(message) = review_intent_matches_draft(
            &review_intent,
            &draft,
            owner,
            funder,
            &policy_check,
            sell_preflight.as_ref(),
        ) {
            return error(
                -3,
                format!("{message}; write revalidate again before posting"),
            );
        }
        let review_intent_hash = blake3_hex(&review_intent_bytes);
        let token_id = match draft.token_id.parse::<U256>() {
            Ok(token_id) => token_id,
            Err(e) => return error(-4, format!("token id parse: {e}")),
        };
        let order = build_order(&OrderParams {
            token_id,
            maker: funder,
            quote: LimitQuote {
                side: draft.side,
                price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                maker_micro: draft.maker_micro,
                taker_micro: draft.taker_micro,
            },
            builder_code: None,
            signature_type: SIG_TYPE_POLY_1271,
        });
        let digest = poly1271_digest(&order, POLYGON, draft.neg_risk);
        let prepared = PreparedSigning::Order(PreparedOrder {
            draft_id: id.into(),
            owner: owner.to_checksum(None),
            funder: funder.to_checksum(None),
            condition_id: draft.condition_id.clone(),
            token_id: draft.token_id.clone(),
            side: order.side,
            price_micro: draft.limit_price_micro,
            size_micro: draft.size_micro,
            maker_amount: order.makerAmount.to_string(),
            taker_amount: order.takerAmount.to_string(),
            order_type: draft.order_type.as_str().into(),
            salt: order.salt.to_string(),
            timestamp_ms: order.timestamp.to_string(),
            signature_type: order.signatureType,
            neg_risk: draft.neg_risk,
            chain_id: POLYGON,
            review_intent_hash: review_intent_hash.clone(),
            signing_hash: format!("{digest:#x}"),
        });
        if let Err(response) = store_prepared_signing(&prepared_key, &prepared) {
            return response;
        }
        let salt = match u64::try_from(order.salt) {
            Ok(salt) => salt,
            Err(_) => return error(-4, "order salt does not fit in u64"),
        };
        draft.salt = Some(salt);
        draft.status = "signing_prepared".into();
        draft.last_error = None;
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/order.json"), &draft, false)
        {
            return error(-4, "failed to store signing-prepared draft");
        }
        if let DispatchResponse::Error { .. } = store_put_json(
            &format!("{base}/post_attempt.json"),
            &serde_json::json!({
                "wallet": wallet,
                "draft_id": id,
                "owner": owner.to_checksum(None),
                "funder": funder.to_checksum(None),
                "salt": salt,
                "review_intent_hash": review_intent_hash.clone(),
                "poly1271_digest_blake3": blake3_hex(digest.as_slice()),
                "prepared_ms": now_millis(),
                "status": "signing_prepared"
            }),
            false,
        ) {
            return error(-4, "failed to store signing-prepared post attempt");
        }
        (order, funder, review_intent_hash, prepared)
    };
    if let Err(response) =
        verify_review_intent(&format!("{base}/review_intent.json"), &review_intent_hash)
    {
        return response;
    }
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let salt = match u64::try_from(order.salt) {
        Ok(salt) => salt,
        Err(_) => return error(-4, "order salt does not fit in u64"),
    };
    let inner_sig = match sign_prepared(wallet, &prepared, &approval_key) {
        Ok(signature) => signature,
        Err(response) => return response,
    };
    let _ = bloom_petal_sdk::store_del(&prepared_key);
    let _ = bloom_petal_sdk::store_del(&approval_key);
    let signature = match wrap_poly1271_signature(&order, &inner_sig, POLYGON, draft.neg_risk) {
        Ok(signature) => signature,
        Err(e) => return polymarket_error(e),
    };
    let order_body = match OrderBody::from_signed(&order, &signature, &creds.key, draft.order_type)
    {
        Ok(body) => body,
        Err(e) => return polymarket_error(e),
    };
    let body_str = match serde_json::to_string(&order_body) {
        Ok(body) => body,
        Err(e) => return error(-4, format!("order body json: {e}")),
    };
    let body_hash = blake3_hex(body_str.as_bytes());
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/post_attempt.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "salt": salt,
            "review_intent_hash": review_intent_hash.clone(),
            "order_body_blake3": body_hash.clone(),
            "signed_ms": now_millis(),
            "status": "signed"
        }),
        false,
    ) {
        return error(-4, "failed to store post attempt");
    }
    draft.status = "signed".into();
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/order.json"), &draft, false)
    {
        return error(-4, "failed to store signed draft");
    }
    let post_result = clob_l2_request_classified(owner, &creds, "POST", "/order", &[], &body_str)
        .and_then(classify_clob_post_success);
    match post_result {
        Ok(raw_response) => {
            let status = clob_response_status(&raw_response);
            let clob_order_id = clob_response_order_id(&raw_response);
            let filled_size_micro = clob_response_filled_size_micro(&raw_response);
            let posted_ms = now_millis();
            let receipt = StoreTradeReceipt {
                draft_id: id.into(),
                wallet: wallet.into(),
                slug: draft.slug.clone(),
                token_id: draft.token_id.clone(),
                side: draft.side,
                order_type: draft.order_type,
                funder: Some(funder.to_checksum(None)),
                signature_type: SIG_TYPE_POLY_1271,
                amount_microusd: draft.amount_micro,
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: clob_order_id.clone(),
                clob_status: status.clone(),
                filled_size_micro,
                raw_response: clob_response_public_summary(
                    &status,
                    &clob_order_id,
                    filled_size_micro,
                ),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(-4, "failed to store receipt");
            }
            draft.status =
                if clob_status_excluded_from_daily_cap(status.as_str(), Some(draft.order_type)) {
                    "rejected".into()
                } else {
                    "posted".into()
                };
            draft.clob_order_id = clob_order_id;
            draft.clob_status = Some(status);
            draft.last_error = None;
            store_put_json(&format!("{base}/order.json"), &draft, false)
        }
        Err(failure) if !failure.ambiguous => {
            let posted_ms = now_millis();
            let receipt = StoreTradeReceipt {
                draft_id: id.into(),
                wallet: wallet.into(),
                slug: draft.slug.clone(),
                token_id: draft.token_id.clone(),
                side: draft.side,
                order_type: draft.order_type,
                funder: Some(funder.to_checksum(None)),
                signature_type: SIG_TYPE_POLY_1271,
                amount_microusd: draft.amount_micro,
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: None,
                clob_status: "rejected".into(),
                filled_size_micro: None,
                raw_response: serde_json::json!({
                    "status": "rejected",
                    "http_status": failure.status,
                    "body": "redacted"
                }),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            draft.status = "rejected".into();
            draft.clob_status = Some("rejected".into());
            draft.last_error = Some("CLOB rejected the signed order".into());
            if let DispatchResponse::Error { .. } =
                store_put_json(&format!("{base}/order.json"), &draft, false)
            {
                return error(
                    -4,
                    "CLOB rejected order and draft state could not be persisted",
                );
            }
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(-4, "CLOB rejected order and receipt could not be persisted");
            }
            failure.response
        }
        Err(failure) => {
            if let Some(raw_response) =
                reconcile_ambiguous_post(owner, &creds, &draft, funder, salt)
            {
                let status = clob_response_status(&raw_response);
                let clob_order_id = clob_response_order_id(&raw_response);
                let filled_size_micro = clob_response_filled_size_micro(&raw_response);
                let posted_ms = now_millis();
                let receipt = StoreTradeReceipt {
                    draft_id: id.into(),
                    wallet: wallet.into(),
                    slug: draft.slug.clone(),
                    token_id: draft.token_id.clone(),
                    side: draft.side,
                    order_type: draft.order_type,
                    funder: Some(funder.to_checksum(None)),
                    signature_type: SIG_TYPE_POLY_1271,
                    amount_microusd: draft.amount_micro,
                    limit_price_micro: draft.limit_price_micro,
                    size_micro: draft.size_micro,
                    salt,
                    clob_order_id: clob_order_id.clone(),
                    clob_status: status.clone(),
                    filled_size_micro,
                    raw_response: clob_reconciled_public_summary(
                        &status,
                        &clob_order_id,
                        filled_size_micro,
                    ),
                    review_intent_hash: Some(review_intent_hash),
                    posted_ms,
                };
                if let DispatchResponse::Error { .. } = store_put_json(
                    &format!("{base}/post_attempt.json"),
                    &serde_json::json!({
                        "wallet": wallet,
                        "draft_id": id,
                        "owner": owner.to_checksum(None),
                        "funder": funder.to_checksum(None),
                        "salt": salt,
                        "review_intent_hash": receipt.review_intent_hash.clone(),
                        "order_body_blake3": body_hash,
                        "reconciled_ms": posted_ms,
                        "status": "reconciled_open_order",
                        "clob_order_id": clob_order_id,
                        "clob_status": status
                    }),
                    false,
                ) {
                    return error(-4, "failed to store reconciled post attempt");
                }
                if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                    return error(-4, "failed to store reconciled receipt");
                }
                draft.status = if clob_status_excluded_from_daily_cap(
                    receipt.clob_status.as_str(),
                    Some(draft.order_type),
                ) {
                    "rejected".into()
                } else {
                    "posted".into()
                };
                draft.clob_order_id = receipt.clob_order_id;
                draft.clob_status = Some(receipt.clob_status);
                draft.last_error = None;
                return store_put_json(&format!("{base}/order.json"), &draft, false);
            }
            let posted_ms = now_millis();
            let receipt = StoreTradeReceipt {
                draft_id: id.into(),
                wallet: wallet.into(),
                slug: draft.slug.clone(),
                token_id: draft.token_id.clone(),
                side: draft.side,
                order_type: draft.order_type,
                funder: Some(funder.to_checksum(None)),
                signature_type: SIG_TYPE_POLY_1271,
                amount_microusd: draft.amount_micro,
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: None,
                clob_status: "ambiguous".into(),
                filled_size_micro: None,
                raw_response: serde_json::json!({
                    "error": "post outcome unknown after signing",
                    "body_hash": body_hash
                }),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            draft.status = "ambiguous".into();
            draft.clob_status = Some("ambiguous".into());
            draft.last_error = Some("post outcome unknown after signing".into());
            if let DispatchResponse::Error { .. } =
                store_put_json(&format!("{base}/order.json"), &draft, false)
            {
                return error(-4, "post outcome ambiguous and failed to store draft state");
            }
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(
                    -4,
                    "post outcome ambiguous and failed to persist receipt/audit",
                );
            }
            let _ = failure;
            error(
                -4,
                "CLOB post outcome unknown after signing; ambiguous receipt written",
            )
        }
    }
}

fn classify_clob_post_success(
    response: serde_json::Value,
) -> Result<serde_json::Value, ClobRequestFailure> {
    if clob_response_order_id(&response).is_some() {
        Ok(response)
    } else {
        Err(ClobRequestFailure {
            ambiguous: true,
            status: Some(200),
            response: error(-4, "CLOB POST /order returned no order id (body redacted)"),
        })
    }
}

fn trade_post_policy_acknowledged(policy_check: &serde_json::Value, confirm_risk: bool) -> bool {
    policy_check
        .get("policy_warn")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
        || confirm_risk
}

fn write_trade_cancel(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid receipt id");
    }
    let req: TradeCancelRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("cancel JSON: {e}")),
    };
    if !req.cancel {
        return error(-3, "cancel must be true");
    }
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let receipt_key = format!("trade/{wallet}/receipts/{id}/receipt.json");
    let Some(bytes) = store_get(&receipt_key) else {
        return error(-1, "receipt not found");
    };
    let mut receipt: StoreTradeReceipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(e) => return error(-4, format!("corrupt receipt: {e}")),
    };
    if receipt.wallet != wallet || receipt.draft_id != id {
        return error(-4, "receipt identity mismatch");
    }
    if receipt.clob_status == "cancelled" {
        if let Err(resp) = mark_trade_draft_cancelled(wallet, id) {
            return resp;
        }
        return DispatchResponse::Write;
    }
    let Some(order_id) = receipt.clob_order_id.clone() else {
        return error(-3, "receipt has no CLOB order id to cancel");
    };
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let body = serde_json::json!({ "orderID": order_id }).to_string();
    let raw = match clob_l2_delete_json(owner, &creds, "/order", &body) {
        Ok(raw) => raw,
        Err(resp) => return resp,
    };
    if !clob_cancel_confirmed(&raw, &order_id) {
        return error(-4, "CLOB cancel response did not confirm cancellation");
    }
    receipt.clob_status = "cancelled".into();
    receipt.raw_response = serde_json::json!({
        "status": "cancelled",
        "order_id": order_id,
        "response_redacted": true
    });
    if let DispatchResponse::Error { .. } = append_trade_audit(
        wallet,
        "order_cancelled",
        serde_json::json!({
            "draft_id": id,
            "clob_order_id": order_id,
        }),
    ) {
        return error(-4, "failed to write cancel audit");
    }
    if let DispatchResponse::Error { .. } = store_put_json(&receipt_key, &receipt, false) {
        return error(-4, "failed to update receipt");
    }
    if let Err(resp) = mark_trade_draft_cancelled(wallet, id) {
        return resp;
    }
    DispatchResponse::Write
}

fn write_discovered_trade_cancel(wallet: &str, order_id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_external_id(order_id) {
        return error(-3, "invalid CLOB order id");
    }
    if let Err(resp) = parse_cancel_confirmation(body) {
        return resp;
    }
    let _lock = match acquire_trade_lock(wallet, order_id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let orders = match clob_l2_get_json(owner, &creds, "/data/orders", &[]) {
        Ok(orders) => orders,
        Err(resp) => return resp,
    };
    if !clob_order_is_discoverable(&orders, order_id) {
        return error(
            -3,
            "CLOB order is not discoverable from account orders; refusing cancellation",
        );
    }
    let request = serde_json::json!({ "orderID": order_id }).to_string();
    let raw = match clob_l2_delete_json(owner, &creds, "/order", &request) {
        Ok(raw) => raw,
        Err(resp) => return resp,
    };
    if !clob_cancel_confirmed(&raw, order_id) {
        return error(-4, "CLOB cancel response did not confirm cancellation");
    }
    if let DispatchResponse::Error { .. } = append_trade_audit(
        wallet,
        "discovered_order_cancelled",
        serde_json::json!({ "clob_order_id": order_id }),
    ) {
        return error(-4, "failed to write cancel audit");
    }
    DispatchResponse::Write
}

fn parse_cancel_confirmation(body: &[u8]) -> Result<(), DispatchResponse> {
    let text = core::str::from_utf8(body)
        .map_err(|_| error(-3, "cancel request body must be UTF-8"))?
        .trim();
    if matches!(text.to_ascii_lowercase().as_str(), "confirm" | "y" | "yes") {
        return Ok(());
    }
    let request: TradeCancelRequest =
        serde_json::from_str(text).map_err(|e| error(-3, format!("cancel JSON: {e}")))?;
    if request.cancel {
        Ok(())
    } else {
        Err(error(-3, "cancel must be true"))
    }
}

fn list_discoverable_clob_order_ids(wallet: &str) -> Result<Vec<String>, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let owner = wallet_address(wallet)?;
    let orders = clob_orders_for_wallet(wallet, owner)?;
    Ok(clob_order_ids(&orders))
}

fn clob_order_ids(orders: &serde_json::Value) -> Vec<String> {
    let rows = orders
        .as_array()
        .or_else(|| orders.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| orders.get("orders").and_then(serde_json::Value::as_array));
    let mut ids = rows
        .into_iter()
        .flatten()
        .filter_map(clob_response_order_id)
        .filter(|id| is_safe_external_id(id))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn clob_order_is_discoverable(orders: &serde_json::Value, order_id: &str) -> bool {
    clob_order_ids(orders).iter().any(|id| id == order_id)
}

fn mark_trade_draft_cancelled(wallet: &str, id: &str) -> Result<(), DispatchResponse> {
    let draft_key = format!("trade/{wallet}/drafts/{id}/order.json");
    if let Some(bytes) = store_get(&draft_key) {
        let mut draft: StoreTradeDraft = match serde_json::from_slice(&bytes) {
            Ok(draft) => draft,
            Err(e) => return Err(error(-4, format!("corrupt draft: {e}"))),
        };
        if draft.wallet != wallet || draft.id != id {
            return Err(error(-4, "draft identity mismatch"));
        }
        draft.status = "cancelled".into();
        draft.clob_status = Some("cancelled".into());
        draft.last_error = None;
        if let DispatchResponse::Error { .. } = store_put_json(&draft_key, &draft, false) {
            return Err(error(-4, "failed to update draft"));
        }
    }
    Ok(())
}

fn write_fund_new(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let req: FundNewRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("fund request JSON: {e}")),
    };
    if req.slippage_bps > 1000 {
        return error(-3, "slippage_bps too high (max 1000)");
    }
    if parse_micro(req.target_pusd.trim()).unwrap_or(0) == 0 {
        return error(-3, "target_pusd must be > 0");
    }
    if !positive_decimal(req.max_spend.trim()) {
        return error(-3, "max_spend must be > 0");
    }
    let id = next_id(&format!("fund/{wallet}/requests/"), ".json");
    let session = StoreFundSession {
        id: id.clone(),
        wallet: wallet.into(),
        target_pusd: req.target_pusd,
        max_spend: req.max_spend,
        from_token: req.from_token.unwrap_or_else(|| "native".into()),
        slippage_bps: req.slippage_bps,
        deposit_wallet: deposit.to_checksum(None),
        deposit_wallet_source: "live_factory_resolved".into(),
        status: "draft".into(),
        prepared_funding: None,
        review_intent: None,
        outbox_ids: Vec::new(),
        outbox_inspections: Vec::new(),
        next_transaction: 0,
        plan_md: None,
        approval: None,
    };
    store_put_json(
        &format!("fund/{wallet}/requests/{id}.json"),
        &session,
        false,
    )
}

fn write_fund_confirm(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !confirmation_body(body) {
        return error(
            -3,
            "fund confirm requires 'confirm', 'y', or {\"confirm\":true}",
        );
    }
    let key = format!("fund/{wallet}/requests/{id}.json");
    let Some(bytes) = store_get(&key) else {
        return error(-1, "not found");
    };
    let mut session: StoreFundSession = match serde_json::from_slice(&bytes) {
        Ok(session) => session,
        Err(e) => return error(-4, format!("corrupt fund request: {e}")),
    };
    if session.wallet != wallet || session.id != id {
        return error(-4, "fund request identity mismatch");
    }
    let prepared = match session.prepared_funding.clone() {
        Some(prepared) => prepared,
        None => match prepare_funding(wallet, &session) {
            Ok(prepared) => {
                session.prepared_funding = Some(prepared.clone());
                session.review_intent = Some(prepared.review_intent.clone());
                session.status = "prepared".into();
                if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
                    return error(-4, "failed to persist prepared funding transaction");
                }
                return DispatchResponse::Write;
            }
            Err(response) => return response,
        },
    };

    if session.next_transaction >= prepared.transactions.len() {
        return store_put_json(&key, &session, false);
    }
    let transaction = &prepared.transactions[session.next_transaction];
    if let Some(outbox_id) = session.outbox_ids.get(session.next_transaction) {
        let inspection = match bloom_petal_sdk::tx_inspect(wallet, "polygon", outbox_id) {
            Ok(inspection) => inspection,
            Err(e) => return sdk_error(e),
        };
        record_fund_inspection(&mut session, &inspection);
        match inspection.state.as_str() {
            "success" => {
                session.next_transaction += 1;
                session.approval = None;
                if session.next_transaction == prepared.transactions.len() {
                    let deposit: Address = match session.deposit_wallet.parse() {
                        Ok(deposit) => deposit,
                        Err(e) => return error(-4, format!("funding deposit wallet: {e}")),
                    };
                    let target = match parse_micro(&session.target_pusd) {
                        Ok(target) => U256::from(target),
                        Err(e) => return error(-4, format!("funding target: {e}")),
                    };
                    let balance = match read_chain_erc20_balance(PUSD, deposit) {
                        Ok(balance) => balance,
                        Err(response) => return response,
                    };
                    session.status = if balance >= target {
                        "complete".into()
                    } else {
                        "confirmed_below_target".into()
                    };
                } else {
                    session.status = "transaction_confirmed".into();
                }
                return store_put_json(&key, &session, false);
            }
            "reverted" | "failed" | "cancelled" => {
                session.status = format!("transaction_{}", inspection.state);
                let _ = store_put_json(&key, &session, false);
                return error(-4, "funding transaction failed; refusing automatic retry");
            }
            "sent" => {
                session.status = "awaiting_confirmation".into();
                return store_put_json(&key, &session, false);
            }
            "pending" => {}
            _ => {
                session.status = "awaiting_confirmation".into();
                return store_put_json(&key, &session, false);
            }
        }
    }
    if session.outbox_ids.len() == session.next_transaction {
        let staged = match bloom_petal_sdk::tx_stage(&EvmTransaction {
            wallet: wallet.into(),
            chain: "polygon".into(),
            to: transaction.to.clone(),
            value_wei: transaction.value_wei.clone(),
            data_hex: transaction.data_hex.clone(),
            nonce: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        }) {
            Ok(staged) => staged,
            Err(e) => return sdk_error(e),
        };
        session.outbox_ids.push(staged.outbox_id.clone());
        session.plan_md = Some(staged.plan_md);
        session.status = "staged".into();
        session.review_intent = Some(prepared.review_intent.clone());
        if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
            return error(
                -4,
                "outbox transaction was staged but its id could not be persisted; refusing automatic restaging",
            );
        }
    }
    let outbox_id = &session.outbox_ids[session.next_transaction];
    let outcome = bloom_petal_sdk::tx_confirm(wallet, "polygon", outbox_id, true);
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => return sdk_error(e),
    };
    session.status = if outcome.approval.is_some() {
        "approval_required".into()
    } else {
        "awaiting_confirmation".into()
    };
    session.plan_md = Some(outcome.plan_md);
    if let Some(approval) = outcome.approval {
        session.approval = Some(ApprovalArtifact {
            action_id: approval.action_id,
            ceremony_url: approval.ceremony_url,
            expires_ms: Some(approval.expires_ms),
            prepared_artifact_digest: prepared.digest(),
            retry_state: "approval_required".into(),
            operation: "fund".into(),
        });
    } else {
        session.approval = None;
    }
    store_put_json(&key, &session, false)
}

fn record_fund_inspection(
    session: &mut StoreFundSession,
    inspection: &bloom_petal_sdk::OutboxInspection,
) {
    let value = serde_json::json!({
        "outbox_id": inspection.outbox_id,
        "state": inspection.state,
        "tx_hash": inspection.tx_hash,
        "receipt": inspection
            .receipt_json
            .as_deref()
            .and_then(|receipt| serde_json::from_str::<serde_json::Value>(receipt).ok()),
    });
    if let Some(existing) = session
        .outbox_inspections
        .iter_mut()
        .find(|existing| existing.get("outbox_id") == value.get("outbox_id"))
    {
        *existing = value;
    } else {
        session.outbox_inspections.push(value);
    }
}

fn prepare_funding(
    wallet: &str,
    session: &StoreFundSession,
) -> Result<PreparedFunding, DispatchResponse> {
    if session.from_token.eq_ignore_ascii_case("pusd") {
        return prepare_direct_pusd_funding(wallet, session);
    }
    let owner = wallet_address(wallet)?;
    let deposit: Address = session
        .deposit_wallet
        .parse()
        .map_err(|e| error(-4, format!("corrupt funding deposit wallet: {e}")))?;
    let target = parse_micro(&session.target_pusd)
        .map(U256::from)
        .map_err(|e| error(-3, format!("target_pusd: {e}")))?;
    let missing = target.saturating_sub(read_chain_erc20_balance(PUSD, deposit)?);
    if !missing.is_zero() && read_chain_erc20_balance(PUSD, owner)? >= missing {
        return prepare_direct_pusd_funding(wallet, session);
    }
    prepare_enso_funding(wallet, session)
}

fn prepare_direct_pusd_funding(
    wallet: &str,
    session: &StoreFundSession,
) -> Result<PreparedFunding, DispatchResponse> {
    let owner = wallet_address(wallet)?;
    let deposit: Address = session
        .deposit_wallet
        .parse()
        .map_err(|e| error(-4, format!("corrupt funding deposit wallet: {e}")))?;
    let target =
        parse_micro(&session.target_pusd).map_err(|e| error(-3, format!("target_pusd: {e}")))?;
    let deposit_balance = read_chain_erc20_balance(PUSD, deposit)?;
    let missing = target.saturating_sub(u64::try_from(deposit_balance).unwrap_or(u64::MAX));
    if missing == 0 {
        return Err(error(
            -3,
            "deposit wallet already meets the requested pUSD target",
        ));
    }
    let owner_balance = read_chain_erc20_balance(PUSD, owner)?;
    if owner_balance < U256::from(missing) {
        return Err(error(
            -3,
            "owner pUSD balance is below the amount needed to reach target",
        ));
    }
    let data_hex = erc20_transfer_calldata(deposit, U256::from(missing));
    let transaction = PreparedEvmTransaction {
        purpose: "direct_pusd_transfer".into(),
        to: PUSD.to_checksum(None),
        value_wei: "0".into(),
        data_hex: data_hex.clone(),
    };
    Ok(PreparedFunding {
        review_intent: serde_json::json!({
            "operation": "polymarket_fund",
            "wallet": wallet,
            "chain": "polygon",
            "from_token": "pUSD",
            "quote_source": "direct_pusd_balance",
            "recipient": deposit.to_checksum(None),
            "target_pusd_micro": target,
            "amount_pusd_micro": missing,
            "max_spend": session.max_spend,
            "max_spend_applies": false,
            "slippage_bps": session.slippage_bps,
            "transactions": [transaction.clone()]
        }),
        transactions: vec![transaction],
    })
}

const ENSO: &str = "https://api.enso.finance";
const ENSO_NATIVE: Address =
    alloy::primitives::address!("EeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE");

fn prepare_enso_funding(
    wallet: &str,
    session: &StoreFundSession,
) -> Result<PreparedFunding, DispatchResponse> {
    let owner = wallet_address(wallet)?;
    let deposit: Address = session
        .deposit_wallet
        .parse()
        .map_err(|e| error(-4, format!("corrupt funding deposit wallet: {e}")))?;
    let target = parse_micro(&session.target_pusd)
        .map(U256::from)
        .map_err(|e| error(-3, format!("target_pusd: {e}")))?;
    let current = read_chain_erc20_balance(PUSD, deposit)?;
    let missing = target.saturating_sub(current);
    if missing.is_zero() {
        return Err(error(
            -3,
            "deposit wallet already meets the requested pUSD target",
        ));
    }

    let (token_in, decimals, native_in) = match session.from_token.to_ascii_lowercase().as_str() {
        "native" | "pol" | "matic" => (ENSO_NATIVE, 18u8, true),
        _ => {
            let token = session
                .from_token
                .parse::<Address>()
                .map_err(|e| error(-3, format!("from_token must be native or an address: {e}")))?;
            if token == PUSD {
                return prepare_direct_pusd_funding(wallet, session);
            }
            let decimals = read_chain_erc20_decimals(token)?;
            (token, decimals, false)
        }
    };
    let max_spend = parse_decimal_units(&session.max_spend, decimals)?;
    if max_spend.is_zero() {
        return Err(error(-3, "max_spend must be > 0"));
    }
    let input_balance = if native_in {
        read_chain_native_balance(owner)?
    } else {
        read_chain_erc20_balance(token_in, owner)?
    };
    if input_balance < max_spend {
        return Err(error(-3, "input balance is below max_spend"));
    }

    let api_key = load_enso_api_key()?;
    let common = [
        ("fromAddress", owner.to_checksum(None)),
        ("chainId", POLYGON.to_string()),
        ("tokenIn", token_in.to_checksum(None)),
        ("tokenOut", PUSD.to_checksum(None)),
        ("slippage", session.slippage_bps.to_string()),
        ("routingStrategy", "router".into()),
        ("receiver", deposit.to_checksum(None)),
    ];
    let mut quote_params = common.to_vec();
    quote_params.push(("amountIn", max_spend.to_string()));
    let quote = enso_get("/api/v1/shortcuts/quote", &quote_params, &api_key)?;
    let out_at_max = json_u256_field(&quote, "amountOut")?;
    if out_at_max < missing {
        return Err(error(-3, "max_spend cannot buy the missing pUSD amount"));
    }
    let required_in = funding_required_input(max_spend, missing, out_at_max);
    let mut route_params = common.to_vec();
    route_params.push(("amountIn", required_in.to_string()));
    let route = enso_get("/api/v1/shortcuts/route", &route_params, &api_key)?;
    let amount_out = json_u256_field(&route, "amountOut")?;
    if amount_out < missing {
        return Err(error(
            -3,
            "Enso route output is below the missing pUSD amount",
        ));
    }
    if route
        .get("route")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hop| hop.get("destinationChainId"))
        .filter_map(parse_json_u64)
        .any(|chain_id| chain_id != POLYGON)
    {
        return Err(error(-3, "cross-chain funding routes are forbidden"));
    }
    let tx = route
        .get("tx")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| error(-4, "Enso route is missing tx"))?;
    let router = json_address_field(tx, "to")?;
    let from = json_address_field(tx, "from")?;
    if router == Address::ZERO || from != owner {
        return Err(error(-3, "Enso route sender or router is invalid"));
    }
    let value = tx
        .get("value")
        .map(json_u256)
        .transpose()?
        .unwrap_or_default();
    if (native_in && value != required_in) || (!native_in && !value.is_zero()) {
        return Err(error(
            -3,
            "Enso route native value does not match its input",
        ));
    }
    let data_hex = tx
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "Enso route is missing calldata"))?;
    let calldata = canonical_hex_bytes(data_hex, "Enso route calldata")?;
    if !calldata
        .windows(20)
        .any(|window| window == deposit.as_slice())
    {
        return Err(error(
            -3,
            "Enso route calldata does not bind the deposit wallet",
        ));
    }

    let mut transactions = Vec::new();
    let allowance = if native_in {
        None
    } else {
        Some(read_chain_erc20_allowance(token_in, owner, router)?)
    };
    if allowance.is_some_and(|allowance| allowance < required_in) {
        transactions.push(PreparedEvmTransaction {
            purpose: "erc20_exact_approval".into(),
            to: token_in.to_checksum(None),
            value_wei: "0".into(),
            data_hex: erc20_approve_calldata(router, required_in),
        });
    }
    transactions.push(PreparedEvmTransaction {
        purpose: "enso_swap".into(),
        to: router.to_checksum(None),
        value_wei: value.to_string(),
        data_hex: data_hex.to_ascii_lowercase(),
    });
    let quote_digest =
        blake3_hex(&serde_json::to_vec(&quote).map_err(|e| error(-4, format!("quote JSON: {e}")))?);
    let route_digest =
        blake3_hex(&serde_json::to_vec(&route).map_err(|e| error(-4, format!("route JSON: {e}")))?);
    let review_intent = serde_json::json!({
        "operation": "polymarket_fund",
        "wallet": wallet,
        "owner": owner.to_checksum(None),
        "chain": "polygon",
        "chain_id": POLYGON,
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": session.deposit_wallet_source,
        "target_pusd_micro": target.to_string(),
        "current_pusd_micro": current.to_string(),
        "missing_pusd_micro": missing.to_string(),
        "input_token": token_in.to_checksum(None),
        "input_decimals": decimals,
        "input_balance": input_balance.to_string(),
        "max_spend": max_spend.to_string(),
        "required_input": required_in.to_string(),
        "slippage_bps": session.slippage_bps,
        "quote_source": "enso",
        "quote_response_digest": quote_digest,
        "route_response_digest": route_digest,
        "route_output_pusd_micro": amount_out.to_string(),
        "router": router.to_checksum(None),
        "prepared_ms": now_millis().to_string(),
        "transactions": transactions,
    });
    Ok(PreparedFunding {
        review_intent,
        transactions,
    })
}

fn enso_get(
    path: &str,
    params: &[(impl AsRef<str>, String)],
    api_key: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    let owned = params
        .iter()
        .map(|(key, value)| (key.as_ref(), value.as_str()))
        .collect::<Vec<_>>();
    let response = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: url_with_query(&format!("{ENSO}{path}"), &owned),
            headers: vec![("authorization".into(), format!("Bearer {api_key}"))],
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&response.status) {
        return Err(error(
            -4,
            format!(
                "Enso request failed with status {} (body redacted, {} bytes)",
                response.status,
                response.body.len()
            ),
        ));
    }
    serde_json::from_slice(&response.body).map_err(|e| error(-4, format!("Enso JSON: {e}")))
}

fn json_u256_field(value: &serde_json::Value, field: &str) -> Result<U256, DispatchResponse> {
    value
        .get(field)
        .ok_or_else(|| error(-4, format!("response is missing {field}")))
        .and_then(json_u256)
}

fn json_u256(value: &serde_json::Value) -> Result<U256, DispatchResponse> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .ok_or_else(|| error(-4, "response integer is not a string or u64"))?;
    if let Some(hex) = raw.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).map_err(|e| error(-4, format!("invalid hex integer: {e}")))
    } else {
        raw.parse::<U256>()
            .map_err(|e| error(-4, format!("invalid decimal integer: {e}")))
    }
}

fn json_address_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Address, DispatchResponse> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, format!("transaction is missing {field}")))?
        .parse()
        .map_err(|e| error(-4, format!("invalid transaction {field}: {e}")))
}

fn canonical_hex_bytes(value: &str, field: &str) -> Result<Vec<u8>, DispatchResponse> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| error(-4, format!("{field} is not 0x-prefixed")))?;
    if hex.len() % 2 != 0 || value != value.to_ascii_lowercase() {
        return Err(error(-4, format!("{field} is not canonical lowercase hex")));
    }
    hex::decode(hex).map_err(|e| error(-4, format!("{field}: {e}")))
}

fn parse_decimal_units(value: &str, decimals: u8) -> Result<U256, DispatchResponse> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > decimals as usize
    {
        return Err(error(-3, "amount has invalid decimal precision"));
    }
    let scale = U256::from(10u8).pow(U256::from(decimals));
    let whole = whole
        .parse::<U256>()
        .map_err(|e| error(-3, format!("amount: {e}")))?;
    let mut padded = fraction.to_string();
    padded.extend(core::iter::repeat_n(
        '0',
        decimals as usize - fraction.len(),
    ));
    let fraction = if padded.is_empty() {
        U256::ZERO
    } else {
        padded
            .parse::<U256>()
            .map_err(|e| error(-3, format!("amount fraction: {e}")))?
    };
    whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| error(-3, "amount overflows uint256"))
}

fn funding_required_input(max_spend: U256, missing: U256, output_at_max: U256) -> U256 {
    if output_at_max.is_zero() {
        return max_spend;
    }
    max_spend
        .saturating_mul(missing)
        .checked_div(output_at_max)
        .unwrap_or(max_spend)
        .saturating_mul(U256::from(102u8))
        .checked_div(U256::from(100u8))
        .unwrap_or(max_spend)
        .min(max_spend)
}

fn read_chain_native_balance(holder: Address) -> Result<U256, DispatchResponse> {
    let result = bloom_petal_sdk::chain_read(
        "polygon",
        "eth_getBalance",
        &serde_json::json!([holder.to_checksum(None), "latest"]).to_string(),
    )
    .map_err(sdk_error)?;
    parse_chain_quantity(&result, "native balance")
}

fn read_chain_erc20_decimals(token: Address) -> Result<u8, DispatchResponse> {
    let value = read_chain_eth_call_u256(token, &[0x31, 0x3c, 0xe5, 0x67], "chain ERC20 decimals")?;
    u8::try_from(value).map_err(|_| error(-4, "ERC20 decimals exceed 255"))
}

fn read_chain_eth_call_u256(
    contract: Address,
    calldata: &[u8],
    field: &str,
) -> Result<U256, DispatchResponse> {
    let result = bloom_petal_sdk::chain_read(
        "polygon",
        "eth_call",
        &serde_json::json!([
            {
                "to": contract.to_checksum(None),
                "data": format!("0x{}", hex::encode(calldata))
            },
            "latest"
        ])
        .to_string(),
    )
    .map_err(sdk_error)?;
    parse_chain_quantity(&result, field)
}

fn parse_chain_quantity(result_json: &str, field: &str) -> Result<U256, DispatchResponse> {
    let result: String =
        serde_json::from_str(result_json).map_err(|e| error(-4, format!("{field} JSON: {e}")))?;
    let hex = result
        .strip_prefix("0x")
        .ok_or_else(|| error(-4, format!("{field} is not hex")))?;
    U256::from_str_radix(if hex.is_empty() { "0" } else { hex }, 16)
        .map_err(|e| error(-4, format!("{field}: {e}")))
}

fn erc20_approve_calldata(spender: Address, amount: U256) -> String {
    let mut bytes = Vec::with_capacity(68);
    bytes.extend_from_slice(&[0x09, 0x5e, 0xa7, 0xb3]);
    let mut encoded_spender = [0u8; 32];
    encoded_spender[12..].copy_from_slice(spender.as_slice());
    bytes.extend_from_slice(&encoded_spender);
    bytes.extend_from_slice(&amount.to_be_bytes::<32>());
    format!("0x{}", hex::encode(bytes))
}

fn erc20_transfer_calldata(to: Address, amount: U256) -> String {
    let mut bytes = Vec::with_capacity(4 + 32 + 32);
    bytes.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]);
    let mut recipient = [0u8; 32];
    recipient[12..].copy_from_slice(to.as_slice());
    bytes.extend_from_slice(&recipient);
    bytes.extend_from_slice(&amount.to_be_bytes::<32>());
    format!("0x{}", hex::encode(bytes))
}

fn confirmation_body(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    if matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "confirm" | "y" | "yes"
    ) {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("confirm").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn positive_decimal(value: &str) -> bool {
    let mut dots = 0usize;
    let mut nonzero = false;
    let mut digits = 0usize;
    for byte in value.bytes() {
        match byte {
            b'.' => dots += 1,
            b'0' => digits += 1,
            b'1'..=b'9' => {
                digits += 1;
                nonzero = true;
            }
            _ => return false,
        }
    }
    dots <= 1 && digits > 0 && nonzero && !value.starts_with('.') && !value.ends_with('.')
}

fn read_fund(wallet: &str, id: &str, file: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let Some(bytes) = store_get(&format!("fund/{wallet}/requests/{id}.json")) else {
        return error(-1, "not found");
    };
    let session: StoreFundSession = match serde_json::from_slice(&bytes) {
        Ok(session) => session,
        Err(e) => return error(-4, format!("corrupt fund request: {e}")),
    };
    match file {
        "request.json" | "status.json" => read_json_value(&session),
        "plan.md" => DispatchResponse::Read(render_fund_plan(&session).into_bytes()),
        "review_intent.json" => match &session.review_intent {
            Some(review) => read_json_value(review),
            None => error(-1, "funding transaction has not been prepared"),
        },
        "approval.json" => match &session.approval {
            Some(approval) => read_json_value(approval),
            None => error(-1, "no funding approval is pending"),
        },
        _ => error(-3, "not a fund file"),
    }
}

fn read_redeem_plan(wallet: &str, slug: &str) -> DispatchResponse {
    let market: Market = match get_json(&format!("{GAMMA}/markets/slug/{slug}")) {
        Ok(market) => market,
        Err(response) => return response,
    };
    if market.condition_id.parse::<B256>().is_err() || market.outcomes.len() != 2 {
        return error(
            -3,
            "redeem requires a resolved binary market with a valid condition id",
        );
    }
    DispatchResponse::Read(
        format!(
            "# Redeem {slug}\n\nWallet: {wallet}\nCondition: {}\nNeg risk: {}\n\nConfirmation persists and signs the exact deposit-wallet relayer batch before submission.\n",
            market.condition_id, market.neg_risk
        )
        .into_bytes(),
    )
}

fn read_revoke_approvals_plan(wallet: &str) -> DispatchResponse {
    DispatchResponse::Read(
        format!(
            "# Revoke Polymarket approvals\n\nWallet: {wallet}\n\nThis revokes the four pUSD allowances and four CTF operator approvals created during onboarding through one persisted deposit-wallet relayer batch.\n"
        )
        .into_bytes(),
    )
}

fn read_withdraw_pusd_plan(wallet: &str) -> DispatchResponse {
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    let balance = match read_chain_erc20_balance(PUSD, deposit) {
        Ok(balance) => balance,
        Err(response) => return response,
    };
    DispatchResponse::Read(
        format!(
            "# Withdraw pUSD\n\nWallet: {wallet}\nDeposit wallet: {}\nOwner recipient: {}\nAvailable pUSD base units: {balance}\n\nWrite {{\"confirm\":true,\"amount\":\"<amount|all>\"}} to persist and sign the exact transfer batch.\n",
            deposit.to_checksum(None), owner.to_checksum(None),
        )
        .into_bytes(),
    )
}

fn write_redeem_confirm(wallet: &str, slug: &str, body: &[u8]) -> DispatchResponse {
    if !confirmation_body(body) {
        return error(-3, "redeem confirm requires explicit confirmation");
    }
    match relayer_terminal_receipt_exists(&format!("actions/{wallet}/redeem/{slug}/receipt.json")) {
        Ok(true) => return DispatchResponse::Write,
        Ok(false) => {}
        Err(response) => return response,
    }
    let market: Market = match get_json(&format!("{GAMMA}/markets/slug/{slug}")) {
        Ok(market) => market,
        Err(response) => return response,
    };
    if market.outcomes.len() != 2 {
        return error(-3, "redeem supports binary markets only");
    }
    let condition = match market.condition_id.parse::<B256>() {
        Ok(condition) => condition,
        Err(e) => return error(-3, format!("market condition id: {e}")),
    };
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    let positions: Vec<Position> = match get_json(&url_with_query(
        &format!("{DATA}/positions"),
        &[("user", &deposit.to_checksum(None))],
    )) {
        Ok(positions) => positions,
        Err(response) => return response,
    };
    if !positions
        .iter()
        .any(|position| position.condition_id == market.condition_id && position.redeemable)
    {
        return error(
            -3,
            "market is not currently redeemable for this deposit wallet",
        );
    }
    let call = redeem_positions_call(condition, market.neg_risk);
    if let Err(response) = preflight_relayer_call(deposit, &call) {
        return response;
    }
    execute_relayer_action(
        wallet,
        &format!("redeem/{slug}"),
        vec![call],
        RelayerPostcondition::None,
        None,
    )
}

fn write_revoke_approvals_confirm(wallet: &str, body: &[u8]) -> DispatchResponse {
    if !confirmation_body(body) {
        return error(
            -3,
            "revoke-approvals confirm requires explicit confirmation",
        );
    }
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    execute_relayer_action(
        wallet,
        "revoke-approvals",
        v2_revoke_calls(),
        RelayerPostcondition::ApprovalsRevoked(deposit),
        None,
    )
}

fn write_withdraw_pusd_confirm(wallet: &str, body: &[u8]) -> DispatchResponse {
    let requested_amount = match withdraw_amount(body) {
        Ok(amount) => amount,
        Err(response) => return response,
    };
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    if let Some(amount) = requested_amount {
        let calls = vec![transfer_amount_call(PUSD, owner, amount)];
        match relayer_action_receipt_matches(wallet, "withdraw-pusd", owner, deposit, &calls) {
            Ok(true) => return DispatchResponse::Write,
            Ok(false) => {}
            Err(response) => return response,
        }
    }
    let balance = match read_chain_erc20_balance(PUSD, deposit) {
        Ok(balance) => balance,
        Err(response) => return response,
    };
    if requested_amount.is_none() && balance == U256::ZERO {
        return match relayer_receipt_has_marker(
            &format!("actions/{wallet}/withdraw-pusd/receipt.json"),
            "withdraw_all",
        ) {
            Ok(true) => DispatchResponse::Write,
            Ok(false) => error(-3, "withdraw amount must be positive"),
            Err(response) => response,
        };
    }
    let withdraw_all = requested_amount.is_none();
    let amount = match requested_amount {
        Some(amount) => amount,
        None => balance,
    };
    if amount == U256::ZERO || amount > balance {
        return error(
            -3,
            "withdraw amount must be positive and no greater than deposit pUSD balance",
        );
    }
    execute_relayer_action(
        wallet,
        "withdraw-pusd",
        vec![transfer_amount_call(PUSD, owner, amount)],
        RelayerPostcondition::None,
        withdraw_all.then_some("withdraw_all"),
    )
}

fn preflight_relayer_call(from: Address, call: &Call) -> Result<(), DispatchResponse> {
    let result = bloom_petal_sdk::chain_read(
        "polygon",
        "eth_call",
        &serde_json::json!([
            {
                "from": from.to_checksum(None),
                "to": call.target.to_checksum(None),
                "value": format!("{:#x}", call.value),
                "data": format!("0x{}", hex::encode(call.data.as_ref()))
            },
            "latest"
        ])
        .to_string(),
    )
    .map_err(sdk_error)?;
    let encoded: String = serde_json::from_str(&result)
        .map_err(|e| error(-4, format!("relayer preflight JSON: {e}")))?;
    canonical_hex_bytes(&encoded, "relayer preflight result")?;
    Ok(())
}

fn withdraw_amount(body: &[u8]) -> Result<Option<U256>, DispatchResponse> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| error(-3, format!("withdraw body JSON: {e}")))?;
    if value.get("confirm").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(error(-3, "withdraw requires confirm=true"));
    }
    let amount = value
        .get("amount")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-3, "withdraw requires amount as a decimal string or 'all'"))?;
    if amount.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    let micro = parse_micro(amount).map_err(|e| error(-3, format!("withdraw amount: {e}")))?;
    Ok(Some(U256::from(micro)))
}

fn list_market_slugs() -> Result<Vec<String>, DispatchResponse> {
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

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, DispatchResponse> {
    let resp = http("GET", url, &[], Vec::new())?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!(
                "polymarket api error (status {}): {}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ),
        ));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))
}

fn clob_auth_request(
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> Result<Credentials, ClobAuthError> {
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: format!("{CLOB}{path}"),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).into(), value.clone()))
                .collect(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(|e| ClobAuthError {
        status: None,
        response: sdk_error(e),
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(ClobAuthError {
            status: Some(resp.status),
            response: error(-4, format!("CLOB auth error (status {})", resp.status)),
        });
    }
    let mut creds: Credentials = serde_json::from_slice(&resp.body).map_err(|e| ClobAuthError {
        status: Some(resp.status),
        response: error(-4, format!("json: {e}")),
    })?;
    creds.nonce = CLOB_AUTH_NONCE;
    Ok(creds)
}

struct ClobAuthError {
    status: Option<u16>,
    response: DispatchResponse,
}

fn load_creds(wallet: &str) -> Result<Credentials, DispatchResponse> {
    let Some(bytes) = store_get(&format!("creds/{wallet}/clob.json")) else {
        return Err(error(
            -3,
            format!("wallet '{wallet}' is not onboarded; write onboard/{wallet}/begin first"),
        ));
    };
    serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt credentials: {e}")))
}

fn load_builder_credentials(wallet: &str) -> Result<Option<BuilderCredentials>, DispatchResponse> {
    match bloom_petal_sdk::store_get(&format!("creds/{wallet}/builder.json"), MAX_STORE_BYTES) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| error(-4, format!("corrupt builder credentials: {e}"))),
        Err(SdkError::Host(HostStatus::NotFound)) => Ok(None),
        Err(e) => Err(sdk_error(e)),
    }
}

fn save_builder_credentials(
    wallet: &str,
    creds: &BuilderCredentials,
) -> Result<(), DispatchResponse> {
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("creds/{wallet}/builder.json"), creds, true)
    {
        return Err(error(-4, "failed to store builder credentials"));
    }
    Ok(())
}

fn delete_builder_credentials(wallet: &str) -> Result<(), DispatchResponse> {
    match bloom_petal_sdk::store_del(&format!("creds/{wallet}/builder.json")) {
        Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => Ok(()),
        Err(e) => Err(sdk_error(e)),
    }
}

fn ensure_builder_credentials(
    wallet: &str,
    owner: Address,
    clob_creds: &Credentials,
) -> Result<BuilderCredentials, DispatchResponse> {
    if let Some(creds) = load_builder_credentials(wallet)? {
        return Ok(creds);
    }
    let raw = clob_l2_post_json(owner, clob_creds, "/auth/builder-api-key", "")?;
    let mut creds: BuilderCredentials =
        serde_json::from_value(raw).map_err(|e| error(-4, format!("builder key JSON: {e}")))?;
    creds.created_at_ms = now_millis();
    creds.source = "clob_l2_auth".into();
    save_builder_credentials(wallet, &creds)?;
    Ok(creds)
}

#[derive(Debug, Clone)]
struct LocalRelayerTx {
    id: String,
    state: String,
    transaction_hash: Option<String>,
}

impl LocalRelayerTx {
    fn is_confirmed(&self) -> bool {
        self.state == "STATE_CONFIRMED"
    }

    fn is_failed(&self) -> bool {
        let state = self.state.to_ascii_uppercase();
        state.contains("FAIL") || state.contains("INVALID")
    }
}

struct RelayerSubmitFailure {
    ambiguous: bool,
    response: DispatchResponse,
}

fn relayer_submit_with_builder_repair_classified(
    wallet: &str,
    owner: Address,
    clob_creds: &Credentials,
    body: serde_json::Value,
) -> Result<LocalRelayerTx, RelayerSubmitFailure> {
    let mut builder =
        ensure_builder_credentials(wallet, owner, clob_creds).map_err(|response| {
            RelayerSubmitFailure {
                ambiguous: false,
                response,
            }
        })?;
    match relayer_submit(&builder, &body) {
        Ok(tx) => Ok(tx),
        Err(RelayerHttpError {
            status: 401 | 403, ..
        }) => {
            let listed = clob_l2_get_json(owner, clob_creds, "/auth/builder-api-key", &[])
                .map_err(|response| RelayerSubmitFailure {
                    ambiguous: false,
                    response,
                })?;
            let stored_key_is_active = builder_key_infos(&listed)
                .iter()
                .any(|key| key.key == builder.key && key.revoked_at.is_none());
            if stored_key_is_active {
                return Err(RelayerSubmitFailure {
                    ambiguous: false,
                    response: error(
                        -4,
                        "relayer rejected an active builder key; refusing destructive key rotation",
                    ),
                });
            }
            delete_builder_credentials(wallet).map_err(|response| RelayerSubmitFailure {
                ambiguous: false,
                response,
            })?;
            builder =
                ensure_builder_credentials(wallet, owner, clob_creds).map_err(|response| {
                    RelayerSubmitFailure {
                        ambiguous: false,
                        response,
                    }
                })?;
            relayer_submit(&builder, &body).map_err(relayer_submit_failure)
        }
        Err(err) => Err(relayer_submit_failure(err)),
    }
}

fn relayer_submit_failure(err: RelayerHttpError) -> RelayerSubmitFailure {
    RelayerSubmitFailure {
        ambiguous: err.ambiguous,
        response: relayer_http_error(err),
    }
}

#[derive(Debug)]
struct RelayerHttpError {
    status: u16,
    body: String,
    ambiguous: bool,
}

fn relayer_submit(
    builder: &BuilderCredentials,
    body: &serde_json::Value,
) -> Result<LocalRelayerTx, RelayerHttpError> {
    let body = serde_json::to_string(body).map_err(|e| RelayerHttpError {
        status: 0,
        body: format!("relayer body JSON: {e}"),
        ambiguous: false,
    })?;
    let headers =
        builder_headers(builder, "POST", "/submit", &body).map_err(|message| RelayerHttpError {
            status: 0,
            body: message,
            ambiguous: false,
        })?;
    let mut headers: Vec<(String, String)> = headers
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    headers.push(("content-type".into(), "application/json".into()));
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "POST".into(),
            url: format!("{RELAYER}/submit"),
            headers,
            body: body.into_bytes(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(|e| RelayerHttpError {
        status: 0,
        body: e.message().to_string(),
        ambiguous: true,
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(RelayerHttpError {
            status: resp.status,
            body: format!(
                "relayer /submit response body redacted ({} bytes)",
                resp.body.len()
            ),
            ambiguous: resp.status >= 500,
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| RelayerHttpError {
            status: resp.status,
            body: format!("relayer submit JSON: {e}"),
            ambiguous: true,
        })?;
    parse_relayer_submit_response(&value).map_err(|body| RelayerHttpError {
        status: resp.status,
        body,
        ambiguous: true,
    })
}

fn relayer_wallet_nonce(owner: Address) -> Result<u64, DispatchResponse> {
    let value = relayer_get_json(&url_with_query(
        &format!("{RELAYER}/nonce"),
        &[("address", &format!("{owner:#x}")), ("type", "WALLET")],
    ))?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .or_else(|| value.get("nonce").and_then(parse_json_u64))
        .ok_or_else(|| error(-4, "relayer /nonce response unparsable (body redacted)"))
}

enum RelayerPoll {
    Confirmed(LocalRelayerTx),
    Pending(LocalRelayerTx),
    Failed(LocalRelayerTx),
}

fn relayer_poll_once(tx: &LocalRelayerTx) -> Result<RelayerPoll, DispatchResponse> {
    let mut cur = relayer_transaction(&tx.id)?;
    bind_relayer_transaction_identity(tx, &mut cur)?;
    if cur.is_confirmed() {
        return Ok(RelayerPoll::Confirmed(cur));
    }
    if cur.is_failed() {
        return Ok(RelayerPoll::Failed(cur));
    }
    Ok(RelayerPoll::Pending(cur))
}

fn bind_relayer_transaction_identity(
    expected: &LocalRelayerTx,
    actual: &mut LocalRelayerTx,
) -> Result<(), DispatchResponse> {
    if actual.id != expected.id {
        return Err(error(
            -4,
            "relayer poll returned a different transaction id",
        ));
    }
    match (&expected.transaction_hash, &actual.transaction_hash) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(error(
                -4,
                "relayer poll returned a different transaction hash",
            ));
        }
        (Some(expected), None) => actual.transaction_hash = Some(expected.clone()),
        _ => {}
    }
    Ok(())
}

fn resume_relayer_transaction(
    id: &str,
    progress: Option<&RelayerActionProgress>,
) -> Result<LocalRelayerTx, DispatchResponse> {
    let expected = LocalRelayerTx {
        id: id.into(),
        state: progress
            .and_then(|progress| progress.relayer_state.clone())
            .unwrap_or_else(|| "STATE_NEW".into()),
        transaction_hash: progress.and_then(|progress| progress.transaction_hash.clone()),
    };
    if progress.is_some_and(|progress| progress.transaction_id.as_deref() != Some(id)) {
        return Err(error(
            -4,
            "onboarding progress transaction id does not match persisted status",
        ));
    }
    let mut current = relayer_transaction(id)?;
    bind_relayer_transaction_identity(&expected, &mut current)?;
    Ok(current)
}

fn relayer_transaction(id: &str) -> Result<LocalRelayerTx, DispatchResponse> {
    let value = relayer_get_json(&url_with_query(
        &format!("{RELAYER}/transaction"),
        &[("id", id)],
    ))?;
    parse_relayer_transaction_response(id, &value).map_err(|message| error(-4, message))
}

fn relayer_get_json(url: &str) -> Result<serde_json::Value, DispatchResponse> {
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!(
                "relayer error (status {}; body redacted, {} bytes)",
                resp.status,
                resp.body.len()
            ),
        ));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("relayer JSON: {e}")))
}

fn relayer_batch_body(
    wallet: &str,
    owner: Address,
    deposit: Address,
    nonce: u64,
    deadline: u64,
) -> Result<serde_json::Value, DispatchResponse> {
    let prepared_key = format!("onboard/{wallet}/prepared_relayer_batch.json");
    let approval_key = format!("onboard/{wallet}/approval.json");
    let review_key = format!("onboard/{wallet}/review_intent.json");
    let prepared = match load_prepared_signing(&prepared_key)? {
        Some(PreparedSigning::RelayerBatch(prepared)) => {
            if prepared.owner != owner.to_checksum(None)
                || prepared.deposit_wallet != deposit.to_checksum(None)
                || prepared.chain_id != POLYGON
            {
                return Err(error(
                    -4,
                    "prepared relayer batch does not match this onboarding request",
                ));
            }
            PreparedSigning::RelayerBatch(prepared)
        }
        Some(_) => {
            return Err(error(
                -4,
                "unexpected prepared signing operation for relayer batch",
            ));
        }
        None => {
            let calls = v2_approval_calls();
            let batch = Batch {
                wallet: deposit,
                nonce: U256::from(nonce),
                deadline: U256::from(deadline),
                calls: calls.clone(),
            };
            let hash = batch_signing_hash(&batch, POLYGON, deposit);
            let prepared_calls = calls
                .iter()
                .map(PreparedCall::from_call)
                .collect::<Vec<_>>();
            let review_intent = relayer_review_intent(
                "onboard-approvals",
                owner,
                deposit,
                &prepared_calls,
                nonce,
                deadline,
                hash,
            );
            let review_intent_hash = store_review_intent(&review_key, &review_intent)?;
            let prepared = PreparedSigning::RelayerBatch(PreparedRelayerBatch {
                owner: owner.to_checksum(None),
                deposit_wallet: deposit.to_checksum(None),
                calls: prepared_calls,
                nonce,
                deadline,
                chain_id: POLYGON,
                signing_hash: format!("{hash:#x}"),
                review_intent_hash,
            });
            store_prepared_signing(&prepared_key, &prepared)?;
            prepared
        }
    };
    let PreparedSigning::RelayerBatch(batch) = &prepared else {
        return Err(error(
            -4,
            "unexpected prepared signing operation for relayer batch",
        ));
    };
    let calls: Vec<Call> = batch
        .calls
        .iter()
        .map(PreparedCall::call)
        .collect::<Result<_, _>>()?;
    let preimage = Batch {
        wallet: deposit,
        nonce: U256::from(batch.nonce),
        deadline: U256::from(batch.deadline),
        calls: calls.clone(),
    };
    if prepared.signing_hash()? != batch_signing_hash(&preimage, batch.chain_id, deposit) {
        return Err(error(
            -4,
            "prepared relayer hash does not match its preimage",
        ));
    }
    verify_review_intent(&review_key, &batch.review_intent_hash)?;
    let signature = format!(
        "0x{}",
        hex::encode(sign_prepared(wallet, &prepared, &approval_key)?)
    );
    let calls_json: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "target": call.target.to_checksum(None),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(call.data.as_ref())),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "type": "WALLET",
        "from": owner.to_checksum(None),
        "to": FACTORY.to_checksum(None),
        "nonce": batch.nonce.to_string(),
        "signature": signature,
        "depositWalletParams": {
            "depositWallet": deposit.to_checksum(None),
            "deadline": batch.deadline.to_string(),
            "calls": calls_json,
        },
    }))
}

fn execute_relayer_action(
    wallet: &str,
    action: &str,
    initial_calls: Vec<Call>,
    postcondition: RelayerPostcondition,
    receipt_marker: Option<&str>,
) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if action
        .split('/')
        .any(|segment| !is_safe_external_id(segment))
    {
        return error(-3, "invalid relayer action id");
    }
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    let base = format!("actions/{wallet}/{action}");
    let prepared_key = format!("{base}/prepared_relayer_batch.json");
    let approval_key = format!("{base}/approval.json");
    let progress_key = format!("{base}/progress.json");
    let review_key = format!("{base}/review_intent.json");
    let expected_calls = initial_calls
        .iter()
        .map(PreparedCall::from_call)
        .collect::<Vec<_>>();
    let request_digest = match relayer_request_digest(action, owner, deposit, &expected_calls) {
        Ok(digest) => digest,
        Err(response) => return response,
    };
    match relayer_receipt_matches(&format!("{base}/receipt.json"), &request_digest) {
        Ok(true) => return DispatchResponse::Write,
        Ok(false) => {}
        Err(response) => return response,
    }
    let prepared = match load_prepared_signing(&prepared_key) {
        Ok(Some(PreparedSigning::RelayerBatch(batch))) => {
            if !prepared_relayer_matches(&batch, owner, deposit, &expected_calls) {
                if store_get(&progress_key).is_none() {
                    let _ = bloom_petal_sdk::store_del(&prepared_key);
                    let _ = bloom_petal_sdk::store_del(&approval_key);
                    let _ = bloom_petal_sdk::store_del(&review_key);
                    return error(
                        -3,
                        "relayer action changed; stale review was discarded, retry to prepare a fresh approval",
                    );
                }
                return error(-4, "persisted relayer batch does not match this action");
            }
            PreparedSigning::RelayerBatch(batch)
        }
        Ok(Some(_)) => return error(-4, "persisted action has the wrong prepared signing type"),
        Ok(None) => {
            let nonce = match relayer_wallet_nonce(owner) {
                Ok(nonce) => nonce,
                Err(response) => return response,
            };
            let deadline = now_secs().saturating_add(BATCH_DEADLINE_SECS);
            let batch = Batch {
                wallet: deposit,
                nonce: U256::from(nonce),
                deadline: U256::from(deadline),
                calls: initial_calls.clone(),
            };
            let hash = batch_signing_hash(&batch, POLYGON, deposit);
            let prepared_calls = expected_calls;
            let review_intent = relayer_review_intent(
                action,
                owner,
                deposit,
                &prepared_calls,
                nonce,
                deadline,
                hash,
            );
            let review_intent_hash = match store_review_intent(&review_key, &review_intent) {
                Ok(hash) => hash,
                Err(response) => return response,
            };
            let prepared = PreparedSigning::RelayerBatch(PreparedRelayerBatch {
                owner: owner.to_checksum(None),
                deposit_wallet: deposit.to_checksum(None),
                calls: prepared_calls,
                nonce,
                deadline,
                chain_id: POLYGON,
                signing_hash: format!("{hash:#x}"),
                review_intent_hash,
            });
            if let Err(response) = store_prepared_signing(&prepared_key, &prepared) {
                return response;
            }
            prepared
        }
        Err(response) => return response,
    };
    let PreparedSigning::RelayerBatch(batch) = &prepared else {
        return error(-4, "unexpected prepared relayer action");
    };
    let calls: Vec<Call> = match batch.calls.iter().map(PreparedCall::call).collect() {
        Ok(calls) => calls,
        Err(response) => return response,
    };
    let preimage = Batch {
        wallet: deposit,
        nonce: U256::from(batch.nonce),
        deadline: U256::from(batch.deadline),
        calls: calls.clone(),
    };
    if prepared.signing_hash().ok() != Some(batch_signing_hash(&preimage, batch.chain_id, deposit))
    {
        return error(
            -4,
            "persisted relayer batch hash does not match its preimage",
        );
    }
    if let Err(response) = verify_review_intent(&review_key, &batch.review_intent_hash) {
        return response;
    }
    let prepared_artifact_digest = match prepared_digest(&prepared) {
        Ok(digest) => digest,
        Err(response) => return response,
    };
    let existing_progress = match load_relayer_action_progress(&progress_key) {
        Ok(progress) => progress,
        Err(response) => return response,
    };
    if let Some(progress) = existing_progress {
        if progress.prepared_artifact_digest != prepared_artifact_digest {
            return error(
                -4,
                "relayer action progress does not match the prepared batch",
            );
        }
        let Some(transaction_id) = progress.transaction_id else {
            return error(
                -4,
                "relayer submission may have been accepted but no transaction id was returned; refusing to sign or submit again until the prior attempt is reconciled",
            );
        };
        let submitted = LocalRelayerTx {
            id: transaction_id,
            state: progress.relayer_state.unwrap_or(progress.phase),
            transaction_hash: progress.transaction_hash,
        };
        return finish_relayer_action(
            &base,
            &prepared_key,
            &approval_key,
            &progress_key,
            &submitted,
            postcondition,
            &request_digest,
            receipt_marker,
        );
    }
    let signature = match sign_prepared(wallet, &prepared, &approval_key) {
        Ok(signature) => signature,
        Err(response) => return response,
    };
    let calls_json: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "target": call.target.to_checksum(None),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(call.data.as_ref())),
            })
        })
        .collect();
    let body = serde_json::json!({
        "type": "WALLET",
        "from": owner.to_checksum(None),
        "to": FACTORY.to_checksum(None),
        "nonce": batch.nonce.to_string(),
        "signature": format!("0x{}", hex::encode(signature)),
        "depositWalletParams": {
            "depositWallet": deposit.to_checksum(None),
            "deadline": batch.deadline.to_string(),
            "calls": calls_json,
        },
    });
    let clob_creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(response) => return response,
    };
    let started = RelayerActionProgress {
        prepared_artifact_digest,
        phase: "submission_started".into(),
        transaction_id: None,
        relayer_state: None,
        transaction_hash: None,
    };
    if let DispatchResponse::Error { .. } = store_put_json(&progress_key, &started, false) {
        return error(-4, "failed to persist relayer submission progress");
    }
    let submitted =
        match relayer_submit_with_builder_repair_classified(wallet, owner, &clob_creds, body) {
            Ok(tx) => tx,
            Err(failure) => {
                if !failure.ambiguous {
                    let _ = bloom_petal_sdk::store_del(&progress_key);
                }
                return failure.response;
            }
        };
    let submitted_progress = RelayerActionProgress {
        prepared_artifact_digest: started.prepared_artifact_digest,
        phase: "submitted".into(),
        transaction_id: Some(submitted.id.clone()),
        relayer_state: Some(submitted.state.clone()),
        transaction_hash: submitted.transaction_hash.clone(),
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&progress_key, &submitted_progress, false)
    {
        return error(
            -4,
            "relayer accepted the action but its transaction id could not be persisted; refusing automatic resubmission",
        );
    }
    finish_relayer_action(
        &base,
        &prepared_key,
        &approval_key,
        &progress_key,
        &submitted,
        postcondition,
        &request_digest,
        receipt_marker,
    )
}

#[derive(Clone, Copy)]
enum RelayerPostcondition {
    None,
    ApprovalsRevoked(Address),
}

fn prepared_relayer_matches(
    batch: &PreparedRelayerBatch,
    owner: Address,
    deposit: Address,
    expected_calls: &[PreparedCall],
) -> bool {
    batch.owner == owner.to_checksum(None)
        && batch.deposit_wallet == deposit.to_checksum(None)
        && batch.chain_id == POLYGON
        && batch.calls == expected_calls
}

fn relayer_request_digest(
    action: &str,
    owner: Address,
    deposit: Address,
    calls: &[PreparedCall],
) -> Result<String, DispatchResponse> {
    serde_json::to_vec(&serde_json::json!({
        "action": action,
        "owner": owner.to_checksum(None),
        "deposit_wallet": deposit.to_checksum(None),
        "calls": calls,
    }))
    .map(|bytes| blake3_hex(&bytes))
    .map_err(|e| error(-4, format!("encode relayer request identity: {e}")))
}

fn relayer_action_receipt_matches(
    wallet: &str,
    action: &str,
    owner: Address,
    deposit: Address,
    calls: &[Call],
) -> Result<bool, DispatchResponse> {
    let prepared_calls = calls
        .iter()
        .map(PreparedCall::from_call)
        .collect::<Vec<_>>();
    let digest = relayer_request_digest(action, owner, deposit, &prepared_calls)?;
    relayer_receipt_matches(&format!("actions/{wallet}/{action}/receipt.json"), &digest)
}

fn relayer_receipt_matches(
    receipt_key: &str,
    request_digest: &str,
) -> Result<bool, DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(receipt_key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(false),
        Err(e) => return Err(sdk_error(e)),
    };
    let receipt: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| error(-4, format!("corrupt relayer receipt: {e}")))?;
    Ok(receipt
        .get("request_digest")
        .and_then(serde_json::Value::as_str)
        == Some(request_digest))
}

fn relayer_terminal_receipt_exists(receipt_key: &str) -> Result<bool, DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(receipt_key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(false),
        Err(e) => return Err(sdk_error(e)),
    };
    let receipt: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| error(-4, format!("corrupt relayer receipt: {e}")))?;
    Ok(receipt
        .get("request_digest")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && receipt.get("status").and_then(serde_json::Value::as_str) == Some("STATE_CONFIRMED"))
}

fn relayer_receipt_has_marker(receipt_key: &str, marker: &str) -> Result<bool, DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(receipt_key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(false),
        Err(e) => return Err(sdk_error(e)),
    };
    let receipt: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| error(-4, format!("corrupt relayer receipt: {e}")))?;
    Ok(receipt
        .get("request_marker")
        .and_then(serde_json::Value::as_str)
        == Some(marker)
        && receipt.get("status").and_then(serde_json::Value::as_str) == Some("STATE_CONFIRMED"))
}

fn finish_relayer_action(
    base: &str,
    prepared_key: &str,
    approval_key: &str,
    progress_key: &str,
    submitted: &LocalRelayerTx,
    postcondition: RelayerPostcondition,
    request_digest: &str,
    receipt_marker: Option<&str>,
) -> DispatchResponse {
    let confirmed = match relayer_poll_once(submitted) {
        Ok(RelayerPoll::Confirmed(tx)) => tx,
        Ok(RelayerPoll::Pending(pending)) => {
            let mut progress = match load_relayer_action_progress(progress_key) {
                Ok(Some(progress)) => progress,
                Ok(None) => return error(-4, "missing relayer progress for pending transaction"),
                Err(response) => return response,
            };
            progress.phase = "pending".into();
            progress.transaction_id = Some(pending.id);
            progress.relayer_state = Some(pending.state);
            progress.transaction_hash = pending
                .transaction_hash
                .or_else(|| submitted.transaction_hash.clone());
            return store_put_json(progress_key, &progress, false);
        }
        Ok(RelayerPoll::Failed(failed)) => {
            let failure = store_put_json(
                &format!("{base}/last_failure.json"),
                &serde_json::json!({
                    "transaction_id": failed.id,
                    "transaction_hash": failed.transaction_hash,
                    "state": failed.state,
                    "request_digest": request_digest,
                }),
                false,
            );
            if matches!(failure, DispatchResponse::Error { .. }) {
                return failure;
            }
            let _ = bloom_petal_sdk::store_del(prepared_key);
            let _ = bloom_petal_sdk::store_del(approval_key);
            let _ = bloom_petal_sdk::store_del(progress_key);
            let review_key = format!("{base}/review_intent.json");
            let _ = bloom_petal_sdk::store_del(&review_key);
            return error(
                -4,
                "relayer transaction failed; stale prepared state was cleared, retry to prepare a fresh approval",
            );
        }
        Err(response) => return response,
    };
    if confirmed.id != submitted.id {
        return error(
            -4,
            "relayer confirmation id does not match submitted transaction",
        );
    }
    if submitted.transaction_hash.is_some()
        && confirmed.transaction_hash != submitted.transaction_hash
    {
        return error(
            -4,
            "relayer confirmation hash does not match submitted transaction",
        );
    }
    if let RelayerPostcondition::ApprovalsRevoked(deposit) = postcondition {
        match read_chain_v2_approvals_revoked(deposit) {
            Ok(true) => {}
            Ok(false) => {
                return error(
                    -4,
                    "relayer confirmed revocation but on-chain authorities remain",
                );
            }
            Err(response) => return response,
        }
    }
    let response = store_put_json(
        &format!("{base}/receipt.json"),
        &serde_json::json!({
            "status": confirmed.state,
            "transaction_id": confirmed.id,
            "transaction_hash": confirmed.transaction_hash,
            "request_digest": request_digest,
            "request_marker": receipt_marker,
        }),
        false,
    );
    if !matches!(response, DispatchResponse::Error { .. }) {
        let _ = bloom_petal_sdk::store_del(prepared_key);
        let _ = bloom_petal_sdk::store_del(approval_key);
        let _ = bloom_petal_sdk::store_del(progress_key);
    }
    response
}

fn load_relayer_action_progress(
    key: &str,
) -> Result<Option<RelayerActionProgress>, DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(e) => return Err(sdk_error(e)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| error(-4, format!("corrupt relayer action progress: {e}")))
}

fn persist_relayer_progress_identity(
    key: &str,
    tx: &LocalRelayerTx,
) -> Result<(), DispatchResponse> {
    let mut progress = load_relayer_action_progress(key)?
        .ok_or_else(|| error(-4, "missing relayer submission progress"))?;
    if progress
        .transaction_id
        .as_deref()
        .is_some_and(|id| id != tx.id)
    {
        return Err(error(
            -4,
            "relayer submission progress transaction id changed",
        ));
    }
    if progress.transaction_hash.is_some()
        && tx.transaction_hash.is_some()
        && progress.transaction_hash != tx.transaction_hash
    {
        return Err(error(
            -4,
            "relayer submission progress transaction hash changed",
        ));
    }
    progress.phase = "pending".into();
    progress.transaction_id = Some(tx.id.clone());
    progress.relayer_state = Some(tx.state.clone());
    if progress.transaction_hash.is_none() {
        progress.transaction_hash = tx.transaction_hash.clone();
    }
    match store_put_json(key, &progress, false) {
        DispatchResponse::Write => Ok(()),
        response => Err(response),
    }
}

fn builder_headers(
    creds: &BuilderCredentials,
    method: &str,
    path: &str,
    body: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let timestamp = now_secs().to_string();
    let signature = builder_hmac_signature(&creds.secret, &timestamp, method, path, body)?;
    Ok(vec![
        ("POLY_BUILDER_API_KEY", creds.key.clone()),
        ("POLY_BUILDER_TIMESTAMP", timestamp),
        ("POLY_BUILDER_PASSPHRASE", creds.passphrase.clone()),
        ("POLY_BUILDER_SIGNATURE", signature),
    ])
}

fn builder_hmac_signature(
    secret: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    use hmac::Mac as _;

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(secret)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(secret))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(secret))
        .map_err(|e| format!("builder secret base64: {e}"))?;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&decoded)
        .map_err(|e| format!("builder hmac key: {e}"))?;
    mac.update(format!("{timestamp}{method}{path}{body}").as_bytes());
    let out = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    Ok(out.replace('+', "-").replace('/', "_"))
}

fn parse_relayer_submit_response(value: &serde_json::Value) -> Result<LocalRelayerTx, String> {
    let id = ["transactionID", "transactionId", "transaction_id", "id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .ok_or_else(|| {
            String::from("relayer /submit response missing transaction id (body redacted)")
        })?;
    let state = value
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("STATE_NEW");
    let transaction_hash = ["transactionHash", "transaction_hash", "txHash", "hash"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string);
    Ok(LocalRelayerTx {
        id: id.into(),
        state: state.into(),
        transaction_hash,
    })
}

fn parse_relayer_transaction_response(
    id: &str,
    value: &serde_json::Value,
) -> Result<LocalRelayerTx, String> {
    let tx = match value {
        serde_json::Value::Array(items) => items
            .iter()
            .find(|item| relayer_tx_id_matches(item, id))
            .ok_or_else(|| format!("relayer /transaction response did not contain id {id}"))?,
        other => {
            let Some(returned_id) = relayer_tx_id(other) else {
                return Err(format!(
                    "relayer /transaction response missing id {id} (body redacted)"
                ));
            };
            if returned_id != id {
                return Err(format!(
                    "relayer /transaction response id did not match {id} (body redacted)"
                ));
            }
            other
        }
    };
    let state = tx
        .get("state")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!("relayer /transaction response for {id} missing state (body redacted)")
        })?;
    let parsed_id = relayer_tx_id(tx).unwrap_or(id);
    let transaction_hash = ["transactionHash", "transaction_hash", "txHash", "hash"]
        .iter()
        .find_map(|key| tx.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string);
    Ok(LocalRelayerTx {
        id: parsed_id.into(),
        state: state.into(),
        transaction_hash,
    })
}

fn relayer_tx_id(value: &serde_json::Value) -> Option<&str> {
    ["transactionID", "transactionId", "transaction_id", "id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
}

fn relayer_tx_id_matches(value: &serde_json::Value, id: &str) -> bool {
    relayer_tx_id(value) == Some(id)
}

fn relayer_http_error(err: RelayerHttpError) -> DispatchResponse {
    if err.status == 401 || err.status == 403 {
        return error(
            -4,
            format!(
                "relayer rejected builder authentication (status {}): {}",
                err.status, err.body
            ),
        );
    }
    if err.status == 0 {
        error(-4, err.body)
    } else {
        error(
            -4,
            format!("relayer error (status {}): {}", err.status, err.body),
        )
    }
}

fn parse_json_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn onboard_in_flight_deadline_ms() -> u128 {
    now_millis().saturating_add((ONBOARD_IN_FLIGHT_TIMEOUT_SECS as u128).saturating_mul(1000))
}

fn dispatch_error_message(resp: &DispatchResponse) -> String {
    match resp {
        DispatchResponse::Error { message, .. } => message.clone(),
        other => format!("{other:?}"),
    }
}

fn clob_l2_get_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, DispatchResponse> {
    clob_l2_request(owner, creds, "GET", path, query, "")
}

fn clob_l2_post_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    body: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    clob_l2_request(owner, creds, "POST", path, &[], body)
}

fn clob_l2_delete_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    body: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    clob_l2_request(owner, creds, "DELETE", path, &[], body)
}

fn clob_l2_request(
    owner: Address,
    creds: &Credentials,
    method: &str,
    path: &str,
    query: &[(&str, &str)],
    body: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    clob_l2_request_classified(owner, creds, method, path, query, body)
        .map_err(|failure| failure.response)
}

struct ClobRequestFailure {
    ambiguous: bool,
    status: Option<u16>,
    response: DispatchResponse,
}

fn clob_l2_request_classified(
    owner: Address,
    creds: &Credentials,
    method: &str,
    path: &str,
    query: &[(&str, &str)],
    body: &str,
) -> Result<serde_json::Value, ClobRequestFailure> {
    let mut timestamp = now_secs();
    for attempt in 0..2 {
        let headers = l2_headers(
            owner,
            &creds.key,
            &creds.passphrase,
            &creds.secret,
            timestamp,
            method,
            path,
            body,
        )
        .map_err(|e| ClobRequestFailure {
            ambiguous: false,
            status: None,
            response: error(-4, e.to_string()),
        })?;
        let response = bloom_petal_sdk::http_fetch(
            &HttpRequest {
                method: method.into(),
                url: url_with_query(&format!("{CLOB}{path}"), query),
                headers: headers
                    .into_iter()
                    .map(|(name, value)| (name.into(), value))
                    .collect(),
                body: body.as_bytes().to_vec(),
            },
            MAX_HTTP_BYTES,
        )
        .map_err(|e| ClobRequestFailure {
            ambiguous: true,
            status: None,
            response: sdk_error(e),
        })?;
        if matches!(response.status, 401 | 403)
            && attempt == 0
            && let Ok(server_timestamp) = clob_server_time()
        {
            timestamp = server_timestamp;
            continue;
        }
        if !(200..300).contains(&response.status) {
            return Err(ClobRequestFailure {
                ambiguous: clob_http_status_is_ambiguous(response.status),
                status: Some(response.status),
                response: error(
                    -4,
                    format!(
                        "CLOB {method} {path} failed with status {} (body redacted, {} bytes)",
                        response.status,
                        response.body.len()
                    ),
                ),
            });
        }
        if response.body.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Ok(serde_json::Value::Null);
        }
        return serde_json::from_slice(&response.body).map_err(|e| ClobRequestFailure {
            ambiguous: true,
            status: Some(response.status),
            response: error(-4, format!("CLOB JSON: {e}")),
        });
    }
    unreachable!("bounded CLOB retry loop always returns")
}

fn clob_http_status_is_ambiguous(status: u16) -> bool {
    status >= 500
}

fn clob_server_time() -> Result<u64, DispatchResponse> {
    let response = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: format!("{CLOB}/time"),
            headers: Vec::new(),
            body: Vec::new(),
        },
        128,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&response.status) {
        return Err(error(-4, format!("CLOB time status {}", response.status)));
    }
    core::str::from_utf8(&response.body)
        .map_err(|_| error(-4, "CLOB time is not UTF-8"))?
        .trim()
        .trim_matches('"')
        .parse::<f64>()
        .map(|timestamp| timestamp as u64)
        .map_err(|_| error(-4, "CLOB time is invalid"))
}

fn wallet_address(wallet: &str) -> Result<Address, DispatchResponse> {
    let path = format!("wallets/{wallet}/address");
    let bytes = bloom_petal_sdk::vfs_read(&path, 128).map_err(sdk_error)?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("wallet address is not utf-8: {e}")))?
        .trim();
    raw.parse::<Address>()
        .map_err(|e| error(-4, format!("wallet address parse: {e}")))
}

fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<bloom_petal_sdk::HttpResponse, DispatchResponse> {
    bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
            body,
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)
}

fn read_store(key: &str) -> DispatchResponse {
    match bloom_petal_sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(SdkError::Host(HostStatus::NotFound)) => error(-1, "not found"),
        Err(e) => sdk_error(e),
    }
}

fn acquire_trade_lock(wallet: &str, draft_id: &str) -> Result<StoreTradeLock, DispatchResponse> {
    let key = format!("trade/{wallet}/.lock");
    for attempt in 0..2 {
        let bytes = trade_lock_body(wallet, draft_id)?;
        match bloom_petal_sdk::store_put_new(&key, &bytes, false) {
            Ok(()) => {
                return Ok(StoreTradeLock {
                    key,
                    expected: bytes,
                });
            }
            Err(SdkError::Host(HostStatus::Denied)) if attempt == 0 => {
                let Some(stale_bytes) = trade_lock_stale_bytes(&key) else {
                    return Err(error(
                        -3,
                        format!("another trade operation holds the lock for wallet '{wallet}'"),
                    ));
                };
                match bloom_petal_sdk::store_del_if_value(&key, &stale_bytes) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => continue,
                    Err(SdkError::Host(HostStatus::Denied)) => {
                        return Err(error(
                            -3,
                            format!(
                                "another trade operation refreshed the lock for wallet '{wallet}'"
                            ),
                        ));
                    }
                    Err(e) => return Err(sdk_error(e)),
                }
            }
            Err(SdkError::Host(HostStatus::Denied)) => {
                return Err(error(
                    -3,
                    format!("another trade operation holds the lock for wallet '{wallet}'"),
                ));
            }
            Err(e) => return Err(sdk_error(e)),
        }
    }
    Err(error(
        -3,
        format!("another trade operation holds the lock for wallet '{wallet}'"),
    ))
}

fn trade_lock_body(wallet: &str, draft_id: &str) -> Result<Vec<u8>, DispatchResponse> {
    let mut token = [0u8; 16];
    let random = bloom_petal_sdk::random_bytes(token.len())
        .map_err(|e| error(-4, format!("trade lock random token: {}", e.message())))?;
    token.copy_from_slice(&random);
    let body = serde_json::json!({
        "wallet": wallet,
        "draft_id": draft_id,
        "acquired_ms": now_millis(),
        "token": hex::encode(token)
    });
    serde_json::to_vec(&body).map_err(|e| error(-4, format!("json: {e}")))
}

fn trade_lock_stale_bytes(key: &str) -> Option<Vec<u8>> {
    match bloom_petal_sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => {
            let stale = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("acquired_ms").and_then(serde_json::Value::as_u64))
                .map(|acquired| now_millis().saturating_sub(acquired as u128) > TRADE_LOCK_STALE_MS)
                .unwrap_or(true);
            stale.then_some(bytes)
        }
        Err(_) => None,
    }
}

struct StoreTradeLock {
    key: String,
    expected: Vec<u8>,
}

impl Drop for StoreTradeLock {
    fn drop(&mut self) {
        let _ = bloom_petal_sdk::store_del_if_value(&self.key, &self.expected);
    }
}

fn store_get(key: &str) -> Option<Vec<u8>> {
    bloom_petal_sdk::store_get(key, MAX_STORE_BYTES).ok()
}

fn store_put_json<T: Serialize>(key: &str, value: &T, secret: bool) -> DispatchResponse {
    let bytes = match serde_json::to_vec_pretty(value) {
        Ok(bytes) => bytes,
        Err(e) => return error(-4, format!("json: {e}")),
    };
    match bloom_petal_sdk::store_put(key, &bytes, secret) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PreparedSigning {
    ClobAuth(PreparedClobAuth),
    Order(PreparedOrder),
    RelayerBatch(PreparedRelayerBatch),
}

impl PreparedSigning {
    fn operation(&self) -> &'static str {
        match self {
            Self::ClobAuth(_) => "clob_auth",
            Self::Order(_) => "order",
            Self::RelayerBatch(_) => "relayer_batch",
        }
    }

    fn intent(&self) -> &'static str {
        match self {
            Self::ClobAuth(_) => "polymarket.clob_auth",
            Self::Order(_) => "polymarket.order.poly1271",
            Self::RelayerBatch(_) => "polymarket.relayer_batch",
        }
    }

    fn owner(&self) -> Result<Address, DispatchResponse> {
        let owner = match self {
            Self::ClobAuth(prepared) => &prepared.owner,
            Self::Order(prepared) => &prepared.owner,
            Self::RelayerBatch(prepared) => &prepared.owner,
        };
        owner
            .parse()
            .map_err(|e| error(-4, format!("corrupt prepared owner: {e}")))
    }

    fn signing_hash(&self) -> Result<B256, DispatchResponse> {
        let encoded = match self {
            Self::ClobAuth(prepared) => &prepared.signing_hash,
            Self::Order(prepared) => &prepared.signing_hash,
            Self::RelayerBatch(prepared) => &prepared.signing_hash,
        };
        encoded
            .parse::<B256>()
            .map_err(|e| error(-4, format!("corrupt prepared signing hash: {e}")))
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, DispatchResponse> {
        serde_json::to_vec(self).map_err(|e| error(-4, format!("encode prepared signing: {e}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreparedClobAuth {
    owner: String,
    nonce: u32,
    timestamp: u64,
    credential_action: String,
    chain_id: u64,
    signing_hash: String,
    review_intent_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreparedOrder {
    draft_id: String,
    owner: String,
    funder: String,
    condition_id: String,
    token_id: String,
    side: u8,
    price_micro: u64,
    size_micro: u64,
    maker_amount: String,
    taker_amount: String,
    order_type: String,
    salt: String,
    timestamp_ms: String,
    signature_type: u8,
    neg_risk: bool,
    chain_id: u64,
    review_intent_hash: String,
    signing_hash: String,
}

impl PreparedOrder {
    fn order(&self) -> Result<Order, DispatchResponse> {
        let funder = self
            .funder
            .parse::<Address>()
            .map_err(|e| error(-4, format!("corrupt prepared order funder: {e}")))?;
        let parse_u256 = |value: &str, field: &str| {
            value
                .parse::<U256>()
                .map_err(|e| error(-4, format!("corrupt prepared order {field}: {e}")))
        };
        Ok(Order {
            salt: parse_u256(&self.salt, "salt")?,
            maker: funder,
            signer: funder,
            tokenId: parse_u256(&self.token_id, "token_id")?,
            makerAmount: parse_u256(&self.maker_amount, "maker_amount")?,
            takerAmount: parse_u256(&self.taker_amount, "taker_amount")?,
            side: self.side,
            signatureType: self.signature_type,
            timestamp: parse_u256(&self.timestamp_ms, "timestamp_ms")?,
            metadata: B256::ZERO,
            builder: B256::ZERO,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreparedCall {
    target: String,
    value: String,
    data: String,
}

impl PreparedCall {
    fn from_call(call: &Call) -> Self {
        Self {
            target: call.target.to_checksum(None),
            value: call.value.to_string(),
            data: format!("0x{}", hex::encode(call.data.as_ref())),
        }
    }

    fn call(&self) -> Result<Call, DispatchResponse> {
        let data = self
            .data
            .strip_prefix("0x")
            .ok_or_else(|| error(-4, "corrupt prepared relayer call data"))?;
        Ok(Call {
            target: self
                .target
                .parse()
                .map_err(|e| error(-4, format!("corrupt prepared relayer target: {e}")))?,
            value: self
                .value
                .parse()
                .map_err(|e| error(-4, format!("corrupt prepared relayer value: {e}")))?,
            data: hex::decode(data)
                .map_err(|e| error(-4, format!("corrupt prepared relayer data: {e}")))?
                .into(),
        })
    }
}

fn relayer_review_intent(
    operation: &str,
    owner: Address,
    deposit: Address,
    calls: &[PreparedCall],
    nonce: u64,
    deadline: u64,
    signing_hash: B256,
) -> serde_json::Value {
    serde_json::json!({
        "operation": operation,
        "owner": owner.to_checksum(None),
        "deposit_wallet": deposit.to_checksum(None),
        "chain_id": POLYGON,
        "nonce": nonce,
        "deadline": deadline,
        "calls": calls,
        "signing_hash": format!("{signing_hash:#x}"),
    })
}

fn store_review_intent(
    key: &str,
    review_intent: &serde_json::Value,
) -> Result<String, DispatchResponse> {
    let bytes = serde_json::to_vec(review_intent)
        .map_err(|e| error(-4, format!("encode review intent: {e}")))?;
    match bloom_petal_sdk::store_put(key, &bytes, false) {
        Ok(()) => Ok(blake3_hex(&bytes)),
        Err(e) => Err(sdk_error(e)),
    }
}

fn verify_review_intent(key: &str, expected_hash: &str) -> Result<(), DispatchResponse> {
    let bytes = bloom_petal_sdk::store_get(key, MAX_STORE_BYTES)
        .map_err(|e| sdk_error_with_context("read review intent", e))?;
    if blake3_hex(&bytes) != expected_hash {
        return Err(error(
            -4,
            "review intent does not match the prepared operation",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreparedRelayerBatch {
    owner: String,
    deposit_wallet: String,
    calls: Vec<PreparedCall>,
    nonce: u64,
    deadline: u64,
    chain_id: u64,
    signing_hash: String,
    review_intent_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RelayerActionProgress {
    prepared_artifact_digest: String,
    phase: String,
    transaction_id: Option<String>,
    relayer_state: Option<String>,
    #[serde(default)]
    transaction_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ApprovalArtifact {
    action_id: String,
    ceremony_url: String,
    expires_ms: Option<u64>,
    prepared_artifact_digest: String,
    retry_state: String,
    operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequired {
    action_id: String,
    ceremony_url: String,
    expires_ms: Option<u64>,
}

fn prepared_digest(prepared: &PreparedSigning) -> Result<String, DispatchResponse> {
    Ok(blake3_hex(&prepared.canonical_bytes()?))
}

fn store_prepared_signing(
    key: &str,
    prepared: &PreparedSigning,
) -> Result<String, DispatchResponse> {
    let digest = prepared_digest(prepared)?;
    match store_put_json(key, prepared, false) {
        DispatchResponse::Write => Ok(digest),
        response => Err(response),
    }
}

fn load_prepared_signing(key: &str) -> Result<Option<PreparedSigning>, DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(e) => return Err(sdk_error(e)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| error(-4, format!("corrupt prepared signing artifact: {e}")))
}

fn parse_approval_required(message: &str) -> Option<ApprovalRequired> {
    if !message.contains("Sealed Approval required") {
        return None;
    }
    let action_id = message
        .split_once("action_id=")?
        .1
        .split_once(';')?
        .0
        .trim();
    let ceremony_url = message.split_once("ceremony_url=")?.1.trim();
    if action_id.is_empty() || ceremony_url.is_empty() {
        return None;
    }
    Some(ApprovalRequired {
        action_id: action_id.to_string(),
        ceremony_url: ceremony_url.to_string(),
        expires_ms: None,
    })
}

fn prepared_sign_request(
    wallet: &str,
    prepared: &PreparedSigning,
) -> Result<SignRequest, DispatchResponse> {
    Ok(SignRequest {
        wallet: wallet.into(),
        hash32: prepared.signing_hash()?.into(),
        purpose: prepared.intent().into(),
    })
}

fn approval_artifact_for(
    prepared: &PreparedSigning,
    message: &str,
) -> Result<Option<ApprovalArtifact>, DispatchResponse> {
    let Some(challenge) = parse_approval_required(message) else {
        return Ok(None);
    };
    Ok(Some(ApprovalArtifact {
        action_id: challenge.action_id,
        ceremony_url: challenge.ceremony_url,
        expires_ms: challenge.expires_ms,
        prepared_artifact_digest: prepared_digest(prepared)?,
        retry_state: "approval_required".into(),
        operation: prepared.operation().into(),
    }))
}

fn sign_prepared(
    wallet: &str,
    prepared: &PreparedSigning,
    approval_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    let request = prepared_sign_request(wallet, prepared)?;
    match bloom_petal_sdk::sign_hash(&request) {
        Ok(SignHashOutcome::Signature(sig)) if sig.len() == 65 => {
            validate_existing_approval_artifact(approval_key, prepared, None)?;
            let signature = format!("0x{}", hex::encode(&sig))
                .parse::<Signature>()
                .map_err(|e| error(-4, format!("sign_hash returned invalid signature: {e}")))?;
            let recovered = signature
                .recover_address_from_prehash(&prepared.signing_hash()?)
                .map_err(|e| error(-4, format!("sign_hash signature recovery failed: {e}")))?;
            if recovered != prepared.owner()? {
                return Err(error(
                    -4,
                    "sign_hash signature does not match prepared owner",
                ));
            }
            Ok(sig)
        }
        Ok(SignHashOutcome::Signature(sig)) => {
            Err(error(-4, format!("sign_hash returned {} bytes", sig.len())))
        }
        Ok(SignHashOutcome::ApprovalRequired {
            action_id,
            ceremony_url,
            expires_ms,
        }) => {
            let artifact = ApprovalArtifact {
                action_id,
                ceremony_url,
                expires_ms: Some(expires_ms),
                prepared_artifact_digest: prepared_digest(prepared)?,
                retry_state: "approval_required".into(),
                operation: prepared.operation().into(),
            };
            validate_existing_approval_artifact(approval_key, prepared, Some(&artifact.action_id))?;
            match store_put_json(approval_key, &artifact, false) {
                DispatchResponse::Write => Err(error(
                    -2,
                    format!("Sealed Approval required; read {approval_key} and retry this write"),
                )),
                response => Err(response),
            }
        }
        Err(SdkError::Message(message)) => {
            let Some(artifact) = approval_artifact_for(prepared, &message)? else {
                return Err(error(-4, message));
            };
            validate_existing_approval_artifact(approval_key, prepared, Some(&artifact.action_id))?;
            match store_put_json(approval_key, &artifact, false) {
                DispatchResponse::Write => Err(error(
                    -2,
                    format!("Sealed Approval required; read {approval_key} and retry this write"),
                )),
                response => Err(response),
            }
        }
        Err(e) => Err(sdk_error(e)),
    }
}

fn validate_existing_approval_artifact(
    approval_key: &str,
    prepared: &PreparedSigning,
    returned_action_id: Option<&str>,
) -> Result<(), DispatchResponse> {
    let bytes = match bloom_petal_sdk::store_get(approval_key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(()),
        Err(e) => return Err(sdk_error(e)),
    };
    let existing: ApprovalArtifact = serde_json::from_slice(&bytes)
        .map_err(|e| error(-4, format!("corrupt approval artifact: {e}")))?;
    approval_artifact_matches(
        &existing,
        &prepared_digest(prepared)?,
        prepared.operation(),
        returned_action_id,
    )
}

fn approval_artifact_matches(
    existing: &ApprovalArtifact,
    prepared_artifact_digest: &str,
    operation: &str,
    returned_action_id: Option<&str>,
) -> Result<(), DispatchResponse> {
    approval_artifact_matches_at(
        existing,
        prepared_artifact_digest,
        operation,
        returned_action_id,
        now_millis() as u64,
    )
}

fn approval_artifact_matches_at(
    existing: &ApprovalArtifact,
    prepared_artifact_digest: &str,
    operation: &str,
    returned_action_id: Option<&str>,
    now_ms: u64,
) -> Result<(), DispatchResponse> {
    if existing.prepared_artifact_digest != prepared_artifact_digest
        || existing.operation != operation
    {
        return Err(error(
            -4,
            "approval artifact does not match prepared operation",
        ));
    }
    if returned_action_id.is_some_and(|action_id| action_id != existing.action_id)
        && existing
            .expires_ms
            .is_none_or(|expires_ms| expires_ms > now_ms)
    {
        return Err(error(
            -4,
            "host returned a different action id for the same prepared operation",
        ));
    }
    Ok(())
}

fn store_trade_receipt(wallet: &str, id: &str, receipt: &StoreTradeReceipt) -> DispatchResponse {
    let audit_resp = append_trade_audit(
        wallet,
        "receipt_written",
        serde_json::json!({
            "draft_id": id,
            "clob_status": receipt.clob_status,
            "amount_microusd": receipt.amount_microusd,
        }),
    );
    if let DispatchResponse::Error { .. } = audit_resp {
        return audit_resp;
    }
    store_put_json(
        &format!("trade/{wallet}/receipts/{id}/receipt.json"),
        receipt,
        false,
    )
}

fn append_trade_audit(wallet: &str, event: &str, details: serde_json::Value) -> DispatchResponse {
    let key = format!("trade/{wallet}/audit.jsonl");
    let mut text = match bloom_petal_sdk::store_get(&key, MAX_STORE_BYTES) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(SdkError::Host(HostStatus::NotFound)) => String::new(),
        Err(e) => return sdk_error(e),
    };
    let line = serde_json::json!({
        "ts_ms": now_millis(),
        "event": event,
        "details": details,
    });
    text.push_str(&line.to_string());
    text.push('\n');
    match bloom_petal_sdk::store_put(&key, text.as_bytes(), false) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

fn clob_response_status(raw: &serde_json::Value) -> String {
    raw.get("status")
        .or_else(|| raw.get("orderStatus"))
        .or_else(|| raw.get("order_status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("posted")
        .to_ascii_lowercase()
}

fn clob_status_excluded_from_daily_cap(status: &str, order_type: Option<OrderType>) -> bool {
    status == "rejected"
        || (status == "unmatched" && order_type.is_some_and(|order_type| !order_type.can_rest()))
}

fn reconcile_ambiguous_post(
    owner: Address,
    creds: &Credentials,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> Option<serde_json::Value> {
    let open_orders = clob_l2_get_json(owner, creds, "/data/orders", &[]).ok()?;
    find_matching_open_order(&open_orders, draft, funder, salt)
}

fn find_matching_open_order(
    raw: &serde_json::Value,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> Option<serde_json::Value> {
    let items = clob_open_order_items(raw);
    if let Some(exact) = items
        .iter()
        .find(|item| open_order_matches_draft(item, draft, funder, salt))
    {
        return Some((*exact).clone());
    }

    let mut fallback = items.into_iter().filter(|item| {
        clob_order_fields(item, &["salt"]).is_empty()
            && saltless_open_order_matches_draft(item, draft, funder)
    });
    match (fallback.next(), fallback.next()) {
        (Some(only), None) => Some(only.clone()),
        _ => None,
    }
}

fn clob_open_order_items(raw: &serde_json::Value) -> Vec<&serde_json::Value> {
    match raw {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(map) => ["orders", "data", "results"]
            .iter()
            .filter_map(|key| map.get(*key))
            .flat_map(clob_open_order_items)
            .collect(),
        _ => Vec::new(),
    }
}

fn saltless_open_order_matches_draft(
    item: &serde_json::Value,
    draft: &StoreTradeDraft,
    funder: Address,
) -> bool {
    if clob_response_order_id(item).is_none()
        || matches!(
            clob_response_status(item).as_str(),
            "rejected" | "cancelled" | "canceled"
        )
    {
        return false;
    }
    let token_matches = clob_order_field_strings(
        item,
        &["asset_id", "assetId", "token_id", "tokenId", "tokenID"],
    )
    .ok()
    .flatten()
    .is_some_and(|values| values.iter().all(|value| value == &draft.token_id));
    let side_matches = !clob_order_fields(item, &["side"]).is_empty()
        && clob_order_fields(item, &["side"])
            .into_iter()
            .all(|value| clob_side_value_matches(value, draft.side).unwrap_or(false));
    let price_matches = clob_order_field_micros(item, &["price"])
        .ok()
        .flatten()
        .is_some_and(|values| values.iter().all(|value| *value == draft.limit_price_micro));
    let size_matches = clob_order_field_micros(item, &["original_size", "originalSize", "size"])
        .ok()
        .flatten()
        .is_some_and(|values| values.iter().all(|value| *value == draft.size_micro));
    let maker_matches = clob_order_field_strings(item, &["maker", "signer", "funder"])
        .ok()
        .flatten()
        .is_none_or(|values| {
            let expected = funder.to_checksum(None);
            values
                .iter()
                .all(|value| address_strings_equal(value, &expected))
        });
    token_matches && side_matches && price_matches && size_matches && maker_matches
}

fn open_order_matches_draft(
    item: &serde_json::Value,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> bool {
    if clob_response_order_id(item).is_none() {
        return false;
    }
    if matches!(
        clob_response_status(item).as_str(),
        "rejected" | "cancelled" | "canceled"
    ) {
        return false;
    }
    let Some(salts) = (match clob_order_field_u64s(item, &["salt"]) {
        Ok(values) => values,
        Err(()) => return false,
    }) else {
        return false;
    };
    if salts.iter().any(|value| *value != salt) {
        return false;
    }

    let mut matched_fields = 0usize;
    if let Some(values) = match clob_order_field_strings(
        item,
        &["asset_id", "assetId", "token_id", "tokenId", "tokenID"],
    ) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| value != &draft.token_id) {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_strings(item, &["maker", "signer", "funder"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        let expected = funder.to_checksum(None);
        if values
            .iter()
            .any(|value| !address_strings_equal(value, &expected))
        {
            return false;
        }
        matched_fields += 1;
    }
    if clob_order_fields(item, &["side"])
        .into_iter()
        .try_fold(false, |_, value| clob_side_value_matches(value, draft.side))
        .unwrap_or(false)
    {
        matched_fields += 1;
    } else if !clob_order_fields(item, &["side"]).is_empty() {
        return false;
    }
    if let Some(values) = match clob_order_field_strings(item, &["orderType", "order_type"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values
            .iter()
            .any(|value| !value.eq_ignore_ascii_case(draft.order_type.as_str()))
        {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_u64s(item, &["makerAmount", "maker_amount"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| *value != draft.maker_micro) {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_u64s(item, &["takerAmount", "taker_amount"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| *value != draft.taker_micro) {
            return false;
        }
        matched_fields += 1;
    }

    matched_fields >= 2
}

fn clob_order_fields<'a>(
    item: &'a serde_json::Value,
    names: &[&str],
) -> Vec<&'a serde_json::Value> {
    let mut values = Vec::new();
    for name in names {
        if let Some(value) = item.get(*name) {
            values.push(value);
        }
    }
    if let Some(order) = item.get("order") {
        for name in names {
            if let Some(value) = order.get(*name) {
                values.push(value);
            }
        }
    }
    values
}

fn clob_order_field_strings(
    item: &serde_json::Value,
    names: &[&str],
) -> Result<Option<Vec<String>>, ()> {
    let mut values = Vec::new();
    for value in clob_order_fields(item, names) {
        match value {
            serde_json::Value::String(s) => values.push(s.clone()),
            serde_json::Value::Number(n) => values.push(n.to_string()),
            _ => return Err(()),
        }
    }
    Ok((!values.is_empty()).then_some(values))
}

fn clob_order_field_u64s(item: &serde_json::Value, names: &[&str]) -> Result<Option<Vec<u64>>, ()> {
    let mut values = Vec::new();
    for value in clob_order_fields(item, names) {
        let Some(parsed) = (match value {
            serde_json::Value::Number(n) => n.as_u64(),
            serde_json::Value::String(s) => s.parse::<u64>().ok(),
            _ => None,
        }) else {
            return Err(());
        };
        values.push(parsed);
    }
    Ok((!values.is_empty()).then_some(values))
}

fn clob_order_field_micros(
    item: &serde_json::Value,
    names: &[&str],
) -> Result<Option<Vec<u64>>, ()> {
    let mut values = Vec::new();
    for value in clob_order_fields(item, names) {
        let raw = match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Number(value) => value.to_string(),
            _ => return Err(()),
        };
        values.push(parse_micro(&raw).map_err(|_| ())?);
    }
    Ok((!values.is_empty()).then_some(values))
}

fn address_strings_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn clob_side_value_matches(value: &serde_json::Value, side: Side) -> Result<bool, bool> {
    let matches = match value {
        serde_json::Value::String(s) => {
            let normalized = s.trim().to_ascii_uppercase();
            match side {
                Side::Buy => normalized == "BUY" || normalized == "0",
                Side::Sell => normalized == "SELL" || normalized == "1",
            }
        }
        serde_json::Value::Number(n) => n
            .as_u64()
            .is_some_and(|value| matches!((value, side), (0, Side::Buy) | (1, Side::Sell))),
        _ => return Err(false),
    };
    matches.then_some(true).ok_or(false)
}

fn clob_response_order_id(raw: &serde_json::Value) -> Option<String> {
    raw.get("orderID")
        .or_else(|| raw.get("orderId"))
        .or_else(|| raw.get("order_id"))
        .or_else(|| raw.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn clob_response_filled_size_micro(raw: &serde_json::Value) -> Option<u64> {
    raw.get("size_matched")
        .or_else(|| raw.get("matched_size"))
        .or_else(|| raw.get("filled_size"))
        .and_then(|value| match value {
            serde_json::Value::String(s) => parse_micro(s).ok(),
            serde_json::Value::Number(n) => n
                .as_f64()
                .and_then(|f| parse_api_float_micro(f, "filled_size").ok()),
            _ => None,
        })
}

fn clob_cancel_confirmed(raw: &serde_json::Value, order_id: &str) -> bool {
    let status_cancelled = raw
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(|status| {
            let status = status.to_ascii_lowercase();
            status == "cancelled" || status == "canceled"
        })
        .unwrap_or(false);
    let status_order_matches = raw
        .get("orderID")
        .or_else(|| raw.get("orderId"))
        .or_else(|| raw.get("order_id"))
        .or_else(|| raw.get("id"))
        .and_then(serde_json::Value::as_str)
        == Some(order_id);
    let listed_cancelled = raw
        .get("canceled")
        .or_else(|| raw.get("cancelled"))
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(|item| item.as_str() == Some(order_id)))
        .unwrap_or(false);
    listed_cancelled || (status_cancelled && status_order_matches)
}

fn clob_response_public_summary(
    status: &str,
    order_id: &Option<String>,
    filled_size_micro: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "order_id": order_id,
        "filled_size_micro": filled_size_micro,
        "response_redacted": true
    })
}

fn clob_reconciled_public_summary(
    status: &str,
    order_id: &Option<String>,
    filled_size_micro: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "order_id": order_id,
        "filled_size_micro": filled_size_micro,
        "reconciled_from": "open_orders",
        "response_redacted": true
    })
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn store_wallets(prefix: &str) -> Vec<String> {
    let Ok(keys) = bloom_petal_sdk::store_list(prefix, MAX_LIST_BYTES) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in keys {
        let rest = key.strip_prefix(prefix).unwrap_or(&key);
        if let Some(wallet) = rest.split('/').next()
            && is_safe_segment(wallet)
            && !out.iter().any(|existing| existing == wallet)
        {
            out.push(wallet.to_string());
        }
    }
    out.sort();
    out
}

fn vfs_wallets_or_store(store_prefix: &str) -> Vec<String> {
    match bloom_petal_sdk::vfs_list("wallets", MAX_LIST_BYTES) {
        Ok(names) => safe_wallet_names(names),
        Err(_) if store_prefix.is_empty() => Vec::new(),
        Err(_) => store_wallets(store_prefix),
    }
}

fn safe_wallet_names(names: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for name in names {
        if name != "new" && is_safe_segment(&name) && !out.iter().any(|existing| existing == &name)
        {
            out.push(name);
        }
    }
    out.sort();
    out
}

fn store_ids(prefix: &str, suffix: &str) -> Vec<String> {
    let Ok(keys) = bloom_petal_sdk::store_list(prefix, MAX_LIST_BYTES) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in keys {
        if let Some(rest) = key.strip_prefix(prefix)
            && let Some(id) = rest.strip_suffix(suffix)
            && !id.contains('/')
            && is_safe_segment(id)
            && !out.iter().any(|existing| existing == id)
        {
            out.push(id.to_string());
        }
    }
    out.sort();
    out
}

fn next_id(prefix: &str, suffix: &str) -> String {
    let next = store_ids(prefix, suffix)
        .into_iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    format!("{next:04}")
}

fn read_json_value<T: Serialize>(value: &T) -> DispatchResponse {
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(e) => error(-4, format!("json: {e}")),
    }
}

fn render_onboard_plan(wallet: &str) -> String {
    format!(
        "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request daemon-keystore signatures for CLOB auth and any required deposit-wallet approval batch, store CLOB and builder credentials in the private petal store, deploy the live-factory deposit wallet when needed, rest at `fund` until pUSD arrives, then approve and sync CLOB buying power before marking the wallet tradeable.\n"
    )
}

fn render_trade_plan(draft: &StoreTradeDraft) -> String {
    format!(
        "# Polymarket order draft {}\n\nWallet: {}\nMarket: {}\nQuestion: {}\nOutcome: {}\nToken: {}\nSide: {:?}\nOrder type: {}\nAmount: {}\nPrice bound: {}\nLimit price: {}\nSize: {}\nStatus: {}\n\nThe draft is live-quoted from Gamma/CLOB and ready for review. Signing and posting are still pending.\n",
        draft.id,
        draft.wallet,
        draft.slug,
        draft.question,
        draft.outcome,
        draft.token_id,
        draft.side,
        draft.order_type.as_str(),
        format_micro(draft.amount_micro),
        format_micro(draft.price_bound_micro),
        format_micro(draft.limit_price_micro),
        format_micro(draft.size_micro),
        draft.status
    )
}

fn render_fund_plan(session: &StoreFundSession) -> String {
    format!(
        "# Polymarket funding request {}\n\nWallet: {}\nReceiver: {} ({})\nTarget pUSD: {}\nMax spend: {}\nFrom token: {}\nSlippage bps: {}\nStatus: {}\n",
        session.id,
        session.wallet,
        session.deposit_wallet,
        session.deposit_wallet_source,
        session.target_pusd,
        session.max_spend,
        session.from_token,
        session.slippage_bps,
        session.status
    )
}

fn validate_relative_path(relative: &str) -> Result<&str, String> {
    if relative.is_empty() {
        return Ok(relative);
    }
    for segment in relative.split('/') {
        if !is_safe_segment(segment) {
            return Err(format!("invalid path segment '{segment}'"));
        }
    }
    Ok(relative)
}

fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !segment.bytes().any(|byte| byte == 0)
}

fn is_safe_external_id(id: &str) -> bool {
    is_safe_segment(id) && !id.contains('/')
}

fn path_kind(relative: &str) -> Option<DispatchEntryKind> {
    let segs = split(relative);
    match (segs.first().copied(), segs.len()) {
        (None, 0) => Some(DispatchEntryKind::Dir),
        (Some("markets"), 1) => Some(DispatchEntryKind::Dir),
        (Some("markets"), 2) => Some(DispatchEntryKind::Dir),
        (Some("markets"), 3) if MARKET_FILES.contains(&segs[2]) => Some(DispatchEntryKind::File),
        (Some("meta"), 1) => Some(DispatchEntryKind::Dir),
        (Some("meta"), 2) if META_FILES.contains(&segs[1]) => Some(DispatchEntryKind::File),
        (Some("search"), 1) => Some(DispatchEntryKind::Dir),
        (Some("search"), 2) => Some(DispatchEntryKind::File),
        (Some("positions"), 1) => Some(DispatchEntryKind::Dir),
        (Some("positions"), 2) => Some(DispatchEntryKind::Dir),
        (Some("positions"), 3) if POSITION_FILES.contains(&segs[2]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("onboard"), 1) => Some(DispatchEntryKind::Dir),
        (Some("onboard"), 2) => Some(DispatchEntryKind::Dir),
        (Some("onboard"), 3) if segs[2] == "begin" => Some(DispatchEntryKind::WritableFile),
        (Some("onboard"), 3) if ONBOARD_FILES.contains(&segs[2]) => Some(DispatchEntryKind::File),
        (Some("account"), 1) => Some(DispatchEntryKind::Dir),
        (Some("account"), 2) => Some(DispatchEntryKind::Dir),
        (Some("account"), 3) if ACCOUNT_FILES.contains(&segs[2]) => Some(DispatchEntryKind::File),
        (Some("builder-keys"), 1) => Some(DispatchEntryKind::Dir),
        (Some("builder-keys"), 2) => Some(DispatchEntryKind::Dir),
        (Some("builder-keys"), 3) if BUILDER_KEY_FILES.contains(&segs[2]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("builder-keys"), 3) if BUILDER_KEY_WRITABLE_FILES.contains(&segs[2]) => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("settings"), 1) => Some(DispatchEntryKind::Dir),
        (Some("settings"), 2) if SETTINGS_WRITABLE_FILES.contains(&segs[1]) => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("fund"), 1) => Some(DispatchEntryKind::Dir),
        (Some("fund"), 2) => Some(DispatchEntryKind::Dir),
        (Some("fund"), 3) if segs[2] == "new" => Some(DispatchEntryKind::WritableFile),
        (Some("fund"), 3) => Some(DispatchEntryKind::Dir),
        (Some("fund"), 4) if FUND_FILES.contains(&segs[3]) => Some(DispatchEntryKind::File),
        (Some("fund"), 4) if FUND_WRITABLE_FILES.contains(&segs[3]) => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("redeem"), 1) => Some(DispatchEntryKind::Dir),
        (Some("redeem"), 2) => Some(DispatchEntryKind::Dir),
        (Some("redeem"), 3) => Some(DispatchEntryKind::Dir),
        (Some("redeem"), 4) if RELAYER_ACTION_FILES.contains(&segs[3]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("redeem"), 4) if RELAYER_ACTION_WRITABLE_FILES.contains(&segs[3]) => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("revoke-approvals"), 1) => Some(DispatchEntryKind::Dir),
        (Some("revoke-approvals"), 2) => Some(DispatchEntryKind::Dir),
        (Some("revoke-approvals"), 3) if segs[2] == "request" => Some(DispatchEntryKind::Dir),
        (Some("revoke-approvals"), 4)
            if segs[2] == "request" && RELAYER_ACTION_FILES.contains(&segs[3]) =>
        {
            Some(DispatchEntryKind::File)
        }
        (Some("revoke-approvals"), 4)
            if segs[2] == "request" && RELAYER_ACTION_WRITABLE_FILES.contains(&segs[3]) =>
        {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("withdraw"), 1) => Some(DispatchEntryKind::Dir),
        (Some("withdraw"), 2) => Some(DispatchEntryKind::Dir),
        (Some("withdraw"), 3) if segs[2] == "pusd" => Some(DispatchEntryKind::Dir),
        (Some("withdraw"), 4) if segs[2] == "pusd" && RELAYER_ACTION_FILES.contains(&segs[3]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("withdraw"), 4)
            if segs[2] == "pusd" && RELAYER_ACTION_WRITABLE_FILES.contains(&segs[3]) =>
        {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("trade"), 1) => Some(DispatchEntryKind::Dir),
        (Some("trade"), 2) => Some(DispatchEntryKind::Dir),
        (Some("trade"), 3) if segs[2] == "new" => Some(DispatchEntryKind::WritableFile),
        (Some("trade"), 3)
            if segs[2] == "drafts" || segs[2] == "orders" || segs[2] == "receipts" =>
        {
            Some(DispatchEntryKind::Dir)
        }
        (Some("trade"), 4)
            if segs[2] == "drafts" || segs[2] == "orders" || segs[2] == "receipts" =>
        {
            Some(DispatchEntryKind::Dir)
        }
        (Some("trade"), 5) if segs[2] == "drafts" && DRAFT_FILES.contains(&segs[4]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("trade"), 5) if segs[2] == "drafts" && DRAFT_WRITABLE_FILES.contains(&segs[4]) => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("trade"), 5) if segs[2] == "receipts" && RECEIPT_FILES.contains(&segs[4]) => {
            Some(DispatchEntryKind::File)
        }
        (Some("trade"), 5)
            if segs[2] == "receipts" && RECEIPT_WRITABLE_FILES.contains(&segs[4]) =>
        {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("trade"), 5) if segs[2] == "orders" && segs[4] == "cancel" => {
            Some(DispatchEntryKind::WritableFile)
        }
        (Some("obligations"), 1) => Some(DispatchEntryKind::Dir),
        (Some("obligations"), 2) if segs[1].ends_with(".json") => Some(DispatchEntryKind::File),
        _ => None,
    }
}

fn split(relative: &str) -> Vec<&str> {
    if relative.is_empty() {
        Vec::new()
    } else {
        relative.split('/').collect()
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).into()).collect()
}

fn child_relative(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.into()
    } else {
        format!("{parent}/{child}")
    }
}

fn entry_name(relative: &str) -> &str {
    relative
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("")
}

fn entry(name: &str, kind: DispatchEntryKind) -> DispatchEntry {
    let mode = match kind {
        DispatchEntryKind::Dir => 0o755,
        DispatchEntryKind::WritableFile => 0o644,
        _ => 0o444,
    };
    DispatchEntry {
        name: name.into(),
        kind,
        size: 0,
        mode,
        ttl_hint_ms: None,
        link_target: None,
    }
}

fn url_with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse(base).expect("hard-coded Polymarket URL must parse");
    for (key, value) in pairs {
        url.query_pairs_mut().append_pair(key, value);
    }
    url.to_string()
}

fn now_secs() -> u64 {
    bloom_petal_sdk::now_ms() / 1000
}

fn now_millis() -> u128 {
    u128::from(bloom_petal_sdk::now_ms())
}

fn sdk_error(e: SdkError) -> DispatchResponse {
    match e {
        SdkError::Host(HostStatus::NotFound) => error(-1, "not found"),
        SdkError::Host(HostStatus::Denied) => error(-2, "denied"),
        SdkError::Host(HostStatus::Invalid) => error(-3, "invalid"),
        SdkError::Host(HostStatus::Backend) => error(-4, "backend error"),
        SdkError::Host(HostStatus::BufferTooSmall { needed }) => {
            error(-5, format!("response too large: needs {needed} bytes"))
        }
        other => error(-4, other.message()),
    }
}

fn sdk_error_with_context(context: &str, e: SdkError) -> DispatchResponse {
    let code = match &e {
        SdkError::Host(HostStatus::NotFound) => -1,
        SdkError::Host(HostStatus::Denied) => -2,
        SdkError::Host(HostStatus::Invalid) => -3,
        SdkError::Host(HostStatus::Backend) => -4,
        SdkError::Host(HostStatus::BufferTooSmall { .. }) => -5,
        _ => -4,
    };
    error(code, format!("{context}: {}", e.message()))
}

fn polymarket_error(e: bloom_polymarket::PolymarketError) -> DispatchResponse {
    match e {
        bloom_polymarket::PolymarketError::Invalid(message) => error(-3, message),
        other => error(-4, other.to_string()),
    }
}

fn error(code: i32, message: impl Into<String>) -> DispatchResponse {
    DispatchResponse::Error {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TradeNewRequest {
    slug: String,
    outcome: String,
    amount: String,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    max_price: Option<String>,
    #[serde(default)]
    min_price: Option<String>,
    #[serde(default)]
    limit_price: Option<String>,
    #[serde(default)]
    order_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LocalWalletPolicy {
    #[serde(default)]
    polymarket: LocalPolymarketPolicy,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalPolymarketPolicy {
    #[serde(default)]
    enabled: bool,
    #[serde(default, with = "local_micro_opt")]
    max_order_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    max_daily_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    require_flag_above_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    max_price: Option<u64>,
    #[serde(default = "default_true")]
    allow_neg_risk: bool,
    #[serde(default)]
    allowed_slugs: BTreeSet<String>,
    #[serde(default)]
    denied_slugs: BTreeSet<String>,
    #[serde(default)]
    allowed_condition_ids: BTreeSet<String>,
    #[serde(default)]
    denied_condition_ids: BTreeSet<String>,
}

impl Default for LocalPolymarketPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_order_usd: None,
            max_daily_usd: None,
            require_flag_above_usd: None,
            max_price: None,
            allow_neg_risk: true,
            allowed_slugs: BTreeSet::new(),
            denied_slugs: BTreeSet::new(),
            allowed_condition_ids: BTreeSet::new(),
            denied_condition_ids: BTreeSet::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

mod local_micro_opt {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            S(String),
            I(i64),
            F(f64),
        }
        match Option::<Raw>::deserialize(d)? {
            None => Ok(None),
            Some(Raw::S(s)) => super::parse_micro(s.trim())
                .map(Some)
                .map_err(D::Error::custom),
            Some(Raw::I(i)) => {
                if i < 0 {
                    return Err(D::Error::custom("USD amount cannot be negative"));
                }
                (i as u64)
                    .checked_mul(1_000_000)
                    .map(Some)
                    .ok_or_else(|| D::Error::custom("USD amount too large"))
            }
            Some(Raw::F(f)) => super::parse_micro(&format!("{f}"))
                .map(Some)
                .map_err(D::Error::custom),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalPolicySide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
struct LocalPolymarketOrderCtx {
    slug: String,
    condition_id: String,
    side: LocalPolicySide,
    amount_microusd: u64,
    limit_price_micro: u64,
    active: bool,
    closed: bool,
    order_book_enabled: bool,
    binary_outcomes: bool,
    neg_risk: bool,
    receipt_store_readable: bool,
    daily_posted_microusd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LocalPolicyOutcome {
    Pass,
    Warn,
    Deny,
}

#[derive(Debug, Clone, Serialize)]
struct LocalPolicyCheck {
    rule: String,
    outcome: LocalPolicyOutcome,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TradeRevalidateRequest {
    revalidate: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TradePostRequest {
    post: bool,
    #[serde(default)]
    confirm_risk: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TradeCancelRequest {
    cancel: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct BuilderKeyRevokeRequest {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreTradeDraft {
    id: String,
    wallet: String,
    slug: String,
    question: String,
    condition_id: String,
    outcome: String,
    token_id: String,
    side: Side,
    order_type: OrderType,
    amount_micro: u64,
    price_bound_micro: u64,
    limit_price: Option<String>,
    marketable: bool,
    limit_price_micro: u64,
    size_micro: u64,
    maker_micro: u64,
    taker_micro: u64,
    tick_micro: u64,
    min_order_size_micro: u64,
    neg_risk: bool,
    active: bool,
    closed: bool,
    order_book_enabled: bool,
    binary_outcomes: bool,
    best_ask_micro: Option<u64>,
    best_bid_micro: Option<u64>,
    book_snapshot_secs: u64,
    status: String,
    #[serde(default)]
    salt: Option<u64>,
    #[serde(default)]
    clob_order_id: Option<String>,
    #[serde(default)]
    clob_status: Option<String>,
    #[serde(default)]
    last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct StoreTradeReceiptPolicy {
    side: Side,
    #[serde(default)]
    order_type: Option<OrderType>,
    amount_microusd: u64,
    clob_status: String,
    posted_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreTradeReceipt {
    draft_id: String,
    wallet: String,
    slug: String,
    token_id: String,
    side: Side,
    order_type: OrderType,
    funder: Option<String>,
    signature_type: u8,
    amount_microusd: u64,
    limit_price_micro: u64,
    size_micro: u64,
    salt: u64,
    clob_order_id: Option<String>,
    clob_status: String,
    filled_size_micro: Option<u64>,
    raw_response: serde_json::Value,
    review_intent_hash: Option<String>,
    posted_ms: u128,
}

#[derive(Debug, Clone)]
struct TradeSnapshot {
    market: Market,
    outcome: String,
    token_id: String,
    neg_risk: bool,
    tick_micro: u64,
    min_size_micro: u64,
    best_ask_micro: Option<u64>,
    best_bid_micro: Option<u64>,
    active: bool,
    closed: bool,
    order_book_enabled: bool,
}

impl TradeSnapshot {
    fn as_shared(&self) -> shared_trade::Snapshot {
        shared_trade::Snapshot {
            market: self.market.clone(),
            token_id: self.token_id.clone(),
            neg_risk: self.neg_risk,
            tick_micro: self.tick_micro,
            min_size_micro: self.min_size_micro,
            best_ask_micro: self.best_ask_micro,
            best_bid_micro: self.best_bid_micro,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FundNewRequest {
    target_pusd: String,
    max_spend: String,
    #[serde(default)]
    from_token: Option<String>,
    #[serde(default = "default_slippage_bps")]
    slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreFundSession {
    id: String,
    wallet: String,
    target_pusd: String,
    max_spend: String,
    from_token: String,
    slippage_bps: u16,
    #[serde(default)]
    deposit_wallet: String,
    #[serde(default)]
    deposit_wallet_source: String,
    status: String,
    #[serde(default)]
    prepared_funding: Option<PreparedFunding>,
    #[serde(default)]
    review_intent: Option<serde_json::Value>,
    #[serde(default)]
    outbox_ids: Vec<String>,
    #[serde(default)]
    outbox_inspections: Vec<serde_json::Value>,
    #[serde(default)]
    next_transaction: usize,
    #[serde(default)]
    plan_md: Option<String>,
    #[serde(default)]
    approval: Option<ApprovalArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedEvmTransaction {
    purpose: String,
    to: String,
    value_wei: String,
    data_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedFunding {
    review_intent: serde_json::Value,
    transactions: Vec<PreparedEvmTransaction>,
}

impl PreparedFunding {
    fn digest(&self) -> String {
        blake3_hex(&serde_json::to_vec(self).expect("prepared EVM transaction always serializes"))
    }
}

fn default_slippage_bps() -> u16 {
    50
}

#[cfg(test)]
mod tests {
    use super::*;

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
            status: "signed".into(),
            salt: Some(42),
            clob_order_id: None,
            clob_status: None,
            last_error: None,
        }
    }

    fn assert_retry_sign_request_is_identical(prepared: PreparedSigning) {
        let persisted = prepared
            .canonical_bytes()
            .expect("serialize prepared state");
        let retry: PreparedSigning =
            serde_json::from_slice(&persisted).expect("deserialize prepared state");
        let first = prepared_sign_request("alice", &prepared).expect("first sign request");
        let second = prepared_sign_request("alice", &retry).expect("retry sign request");

        assert_eq!(persisted, retry.canonical_bytes().unwrap());
        assert_eq!(first.wallet, second.wallet);
        assert_eq!(first.purpose, second.purpose);
        assert_eq!(first.hash32, second.hash32);
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
    fn path_shapes_are_static_and_expected() {
        assert_eq!(path_kind(""), Some(DispatchEntryKind::Dir));
        assert_eq!(path_kind("markets/foo"), Some(DispatchEntryKind::Dir));
        assert_eq!(
            path_kind("markets/foo/book.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(path_kind("meta"), Some(DispatchEntryKind::Dir));
        assert_eq!(path_kind("meta/parity.json"), Some(DispatchEntryKind::File));
        assert_eq!(
            path_kind("meta/route-contract.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("onboard/alice/begin"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("onboard/alice/approval.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("onboard/alice/review_intent.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("trade/alice/drafts/0001/plan.md"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("trade/alice/drafts/0001/revalidate"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("trade/alice/drafts/0001/approval.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("trade/alice/receipts/0001/receipt.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("trade/alice/receipts/0001/cancel"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("account/alice/status.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("account/alice/buying_power.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("account/alice/funding_options.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("fund/alice/0001/confirm"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("fund/alice/0001/approval.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("fund/alice/0001/review_intent.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("settings/enso-api-key"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("builder-keys/alice/keys.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("builder-keys/alice/revoke"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("trade/alice/orders/clob-123/cancel"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("obligations/alice.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("redeem/alice/example/confirm"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(
            path_kind("redeem/alice/example/approval.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("redeem/alice/example/review_intent.json"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("revoke-approvals/alice/request/plan.md"),
            Some(DispatchEntryKind::File)
        );
        assert_eq!(
            path_kind("withdraw/alice/pusd/confirm"),
            Some(DispatchEntryKind::WritableFile)
        );
        assert_eq!(path_kind("trade/alice/new/extra"), None);
    }

    #[test]
    fn route_contract_declares_complete_generic_ipc_surface() {
        let DispatchResponse::Read(bytes) = read_meta("route-contract.json") else {
            panic!("route contract must be readable");
        };
        let contract: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let expected_routes = [
            "markets/<slug>/market.json",
            "markets/<slug>/book.json",
            "markets/<slug>/prices.json",
            "search/<query>",
            "positions/<wallet>/positions.json",
            "positions/<wallet>/trades.json",
            "positions/<wallet>/activity.json",
            "onboard/<wallet>/plan.md",
            "onboard/<wallet>/status.json",
            "onboard/<wallet>/approvals.json",
            "onboard/<wallet>/review_intent.json",
            "account/<wallet>/status.json",
            "account/<wallet>/portfolio.json",
            "account/<wallet>/orders.json",
            "account/<wallet>/buying_power.json",
            "account/<wallet>/funding_options.json",
            "builder-keys/<wallet>/keys.json",
            "builder-keys/<wallet>/revoke",
            "settings/enso-api-key",
            "trade/<wallet>/new",
            "trade/<wallet>/drafts/<id>/plan.md",
            "trade/<wallet>/drafts/<id>/order.json",
            "trade/<wallet>/drafts/<id>/quote.json",
            "trade/<wallet>/drafts/<id>/policy_check.json",
            "trade/<wallet>/drafts/<id>/revalidate",
            "trade/<wallet>/drafts/<id>/review_intent.json",
            "trade/<wallet>/drafts/<id>/post_attempt.json",
            "trade/<wallet>/drafts/<id>/post",
            "trade/<wallet>/receipts/<id>/receipt.json",
            "trade/<wallet>/receipts/<id>/cancel",
            "fund/<wallet>/<id>/confirm",
            "fund/<wallet>/new",
            "fund/<wallet>/<id>/plan.md",
            "fund/<wallet>/<id>/request.json",
            "fund/<wallet>/<id>/status.json",
            "fund/<wallet>/<id>/review_intent.json",
            "fund/<wallet>/<id>/approval.json",
            "trade/<wallet>/orders/<clob-order-id>/cancel",
            "trade/<wallet>/drafts/<id>/approval.json",
            "onboard/<wallet>/begin",
            "onboard/<wallet>/approval.json",
            "redeem/<wallet>/<slug>/plan.md",
            "redeem/<wallet>/<slug>/review_intent.json",
            "redeem/<wallet>/<slug>/approval.json",
            "redeem/<wallet>/<slug>/confirm",
            "redeem/<wallet>/<slug>/receipt.json",
            "revoke-approvals/<wallet>/request/plan.md",
            "revoke-approvals/<wallet>/request/review_intent.json",
            "revoke-approvals/<wallet>/request/approval.json",
            "revoke-approvals/<wallet>/request/confirm",
            "revoke-approvals/<wallet>/request/receipt.json",
            "withdraw/<wallet>/pusd/plan.md",
            "withdraw/<wallet>/pusd/review_intent.json",
            "withdraw/<wallet>/pusd/approval.json",
            "withdraw/<wallet>/pusd/confirm",
            "withdraw/<wallet>/pusd/receipt.json",
            "obligations/<wallet>.json",
        ];
        let routes = contract["routes"].as_object().unwrap();
        let actual_routes: BTreeSet<&str> = routes
            .values()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(actual_routes.len(), expected_routes.len());
        let build_script = include_str!("../../scripts/build.sh");
        for pattern in expected_routes {
            assert!(actual_routes.contains(pattern), "missing route {pattern}");
            let concrete = pattern
                .replace("<wallet>", "alice")
                .replace("<slug>", "example")
                .replace("<query>", "example")
                .replace("<clob-order-id>", "clob-123")
                .replace("<id>", "0001");
            assert!(path_kind(&concrete).is_some(), "unroutable {pattern}");
            let component_path = pattern.replace('<', "[").replace('>', "]");
            assert!(
                build_script.contains(&format!("'{component_path}'")),
                "route missing from authoritative build list: {pattern}"
            );
        }
        let ipc = contract["generic_ipc_only"].as_array().unwrap();
        assert_eq!(ipc.len(), 3);
        assert!(ipc.iter().all(|entry| {
            !entry
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("polymarket")
        }));
    }

    #[test]
    fn discovered_order_ids_accept_account_response_shapes_and_reject_unsafe_ids() {
        let orders = serde_json::json!({
            "data": [
                {"id": "clob-2"},
                {"orderID": "clob-1"},
                {"id": "clob-2"},
                {"id": "../escape"}
            ]
        });

        assert_eq!(clob_order_ids(&orders), vec!["clob-1", "clob-2"]);
        assert!(clob_order_is_discoverable(&orders, "clob-1"));
        assert!(!clob_order_is_discoverable(&orders, "missing"));
    }

    #[test]
    fn cancel_confirmation_accepts_explicit_acknowledgements() {
        assert!(parse_cancel_confirmation(b"confirm").is_ok());
        assert!(parse_cancel_confirmation(br#"{"cancel":true}"#).is_ok());
        assert!(parse_cancel_confirmation(br#"{"cancel":false}"#).is_err());
        assert!(parse_cancel_confirmation(b"").is_err());
    }

    #[test]
    fn builder_key_projection_is_redacted_and_revoke_parser_is_bounded() {
        let infos = builder_key_infos(&serde_json::json!({
            "data": [
                {"key": "key-a", "created_at": "2026-07-09", "revoked_at": null},
                "key-b"
            ]
        }));
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].key, "key-a");
        assert_eq!(infos[1].key, "key-b");

        assert_eq!(parse_builder_key_revoke(b"confirm").unwrap(), None);
        assert_eq!(
            parse_builder_key_revoke(br#"{"confirm":true,"key":"key-a"}"#).unwrap(),
            Some("key-a".into())
        );
        assert!(parse_builder_key_revoke(br#"{"confirm":false}"#).is_err());
        assert!(parse_builder_key_revoke(br#"{"confirm":true,"key":"../key"}"#).is_err());
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
        assert!(!clob_manifest_allows("post", "/data/orders"));
        assert!(!clob_manifest_allows("delete", "/balance-allowance/update"));
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
    fn open_order_reconciliation_prefers_salt_and_requires_unique_stable_fallback() {
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

        let saltless = serde_json::json!({
            "id": "order-fallback",
            "status": "live",
            "maker": funder.to_checksum(None),
            "asset_id": "111",
            "side": "BUY",
            "price": format_micro(draft.limit_price_micro),
            "original_size": format_micro(draft.size_micro)
        });
        assert_eq!(
            find_matching_open_order(
                &serde_json::json!({"data": [saltless.clone()]}),
                &draft,
                funder,
                42
            )
            .and_then(|order| clob_response_order_id(&order)),
            Some("order-fallback".into())
        );
        assert!(
            find_matching_open_order(
                &serde_json::json!({"data": [saltless.clone(), saltless]}),
                &draft,
                funder,
                42
            )
            .is_none()
        );

        let malformed_salt = serde_json::json!({
            "id": "order-malformed",
            "status": "live",
            "salt": "not-a-number",
            "asset_id": "111",
            "side": "BUY",
            "price": format_micro(draft.limit_price_micro),
            "original_size": format_micro(draft.size_micro)
        });
        assert!(
            find_matching_open_order(
                &serde_json::json!({"data": [malformed_salt]}),
                &draft,
                funder,
                42
            )
            .is_none()
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
        let policy: LocalWalletPolicy = toml::from_str("").unwrap();
        let checks = evaluate_local_polymarket_order(&policy.polymarket, &policy_ctx());
        assert!(local_policy_has_deny(&checks));
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.enabled" && check.outcome == LocalPolicyOutcome::Deny
        }));
    }

    #[test]
    fn local_policy_parses_decimal_caps() {
        let policy: LocalWalletPolicy = toml::from_str(
            r#"
[polymarket]
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
        assert!(policy.polymarket.enabled);
        assert_eq!(policy.polymarket.max_order_usd, Some(10_000_000));
        assert_eq!(policy.polymarket.max_daily_usd, Some(25_500_000));
        assert_eq!(policy.polymarket.require_flag_above_usd, Some(5_000_000));
        assert_eq!(policy.polymarket.max_price, Some(750_000));
        assert!(!policy.polymarket.allow_neg_risk);
        assert!(policy.polymarket.denied_slugs.contains("blocked-market"));

        let float_policy: LocalWalletPolicy =
            toml::from_str("[polymarket]\nenabled = true\nmax_price = 0.1\n").unwrap();
        assert_eq!(float_policy.polymarket.max_price, Some(100_000));
    }

    #[test]
    fn local_policy_daily_cap_fails_closed_when_receipts_unknown() {
        let policy: LocalWalletPolicy = toml::from_str(
            r#"
[polymarket]
enabled = true
max_daily_usd = "100"
"#,
        )
        .unwrap();
        let mut ctx = policy_ctx();
        ctx.receipt_store_readable = false;
        let checks = evaluate_local_polymarket_order(&policy.polymarket, &ctx);
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.max_daily_usd" && check.outcome == LocalPolicyOutcome::Deny
        }));

        ctx.receipt_store_readable = true;
        ctx.daily_posted_microusd = None;
        let checks = evaluate_local_polymarket_order(&policy.polymarket, &ctx);
        assert!(checks.iter().any(|check| {
            check.rule == "polymarket.max_daily_usd" && check.outcome == LocalPolicyOutcome::Deny
        }));
    }

    #[test]
    fn sealed_retry_reuses_clob_auth_timestamp_hash_and_preimage() {
        let owner: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
            .parse()
            .unwrap();
        let timestamp = 1_762_000_001;
        let hash = clob_auth_signing_hash(owner, timestamp, CLOB_AUTH_NONCE, POLYGON);
        let prepared = PreparedSigning::ClobAuth(PreparedClobAuth {
            owner: owner.to_checksum(None),
            nonce: CLOB_AUTH_NONCE,
            timestamp,
            credential_action: "mint_or_derive".into(),
            chain_id: POLYGON,
            signing_hash: format!("{hash:#x}"),
            review_intent_hash: "review-digest".into(),
        });

        assert_retry_sign_request_is_identical(prepared);
    }

    #[test]
    fn sealed_retry_reuses_order_salt_timestamp_hash_and_preimage() {
        let owner: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
            .parse()
            .unwrap();
        let order = Order {
            salt: U256::from(42),
            maker: owner,
            signer: owner,
            tokenId: U256::from(111),
            makerAmount: U256::from(1_000_000),
            takerAmount: U256::from(11_111_100),
            side: Side::Buy as u8,
            signatureType: SIG_TYPE_POLY_1271,
            timestamp: U256::from(1_762_000_001_234u64),
            metadata: B256::ZERO,
            builder: B256::ZERO,
        };
        let hash = poly1271_digest(&order, POLYGON, false);
        let prepared = PreparedSigning::Order(PreparedOrder {
            draft_id: "0001".into(),
            owner: owner.to_checksum(None),
            funder: owner.to_checksum(None),
            condition_id: "0xcondition".into(),
            token_id: order.tokenId.to_string(),
            side: order.side,
            price_micro: 90_000,
            size_micro: 11_111_100,
            maker_amount: order.makerAmount.to_string(),
            taker_amount: order.takerAmount.to_string(),
            order_type: "FAK".into(),
            salt: order.salt.to_string(),
            timestamp_ms: order.timestamp.to_string(),
            signature_type: order.signatureType,
            neg_risk: false,
            chain_id: POLYGON,
            review_intent_hash: "review-digest".into(),
            signing_hash: format!("{hash:#x}"),
        });

        let reconstructed = match &prepared {
            PreparedSigning::Order(prepared) => prepared.order().unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(order, reconstructed);
        assert_eq!(poly1271_digest(&reconstructed, POLYGON, false), hash);
        assert_retry_sign_request_is_identical(prepared);
    }

    #[test]
    fn sealed_retry_reuses_relayer_calls_nonce_deadline_hash_and_preimage() {
        let owner: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
            .parse()
            .unwrap();
        let recipient: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        for (operation, calls) in [
            ("onboard", v2_approval_calls()),
            (
                "redeem",
                vec![redeem_positions_call(B256::repeat_byte(1), false)],
            ),
            ("revoke", v2_revoke_calls()),
            (
                "withdraw",
                vec![transfer_amount_call(PUSD, recipient, U256::from(7u64))],
            ),
        ] {
            let batch = Batch {
                wallet: owner,
                nonce: U256::from(7),
                deadline: U256::from(1_762_003_600u64),
                calls: calls.clone(),
            };
            let hash = batch_signing_hash(&batch, POLYGON, owner);
            let prepared = PreparedSigning::RelayerBatch(PreparedRelayerBatch {
                owner: owner.to_checksum(None),
                deposit_wallet: owner.to_checksum(None),
                calls: calls.iter().map(PreparedCall::from_call).collect(),
                nonce: 7,
                deadline: 1_762_003_600,
                chain_id: POLYGON,
                signing_hash: format!("{hash:#x}"),
                review_intent_hash: format!("{operation}-review-digest"),
            });

            let PreparedSigning::RelayerBatch(retry) = &prepared else {
                unreachable!();
            };
            let reconstructed = Batch {
                wallet: owner,
                nonce: U256::from(retry.nonce),
                deadline: U256::from(retry.deadline),
                calls: retry
                    .calls
                    .iter()
                    .map(PreparedCall::call)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
            };
            let reconstructed_calls: Vec<PreparedCall> = reconstructed
                .calls
                .iter()
                .map(PreparedCall::from_call)
                .collect();
            assert_eq!(reconstructed.nonce, batch.nonce, "{operation}");
            assert_eq!(reconstructed.deadline, batch.deadline, "{operation}");
            assert_eq!(reconstructed_calls, retry.calls, "{operation}");
            assert_eq!(
                batch_signing_hash(&reconstructed, POLYGON, owner),
                hash,
                "{operation}"
            );
            assert!(prepared_relayer_matches(
                retry,
                owner,
                owner,
                &reconstructed_calls
            ));
            let mut changed_calls = reconstructed_calls;
            changed_calls[0].data.push_str("00");
            assert!(!prepared_relayer_matches(
                retry,
                owner,
                owner,
                &changed_calls
            ));
            assert_retry_sign_request_is_identical(prepared);
        }
    }

    #[test]
    fn malformed_relayer_responses_are_always_redacted() {
        let secret = "raw-owner-signature-must-not-leak";
        let submit = parse_relayer_submit_response(&serde_json::json!({
            "signature": secret,
            "body": {"signature": secret}
        }))
        .unwrap_err();
        assert!(submit.contains("body redacted"));
        assert!(!submit.contains(secret));

        let transaction = parse_relayer_transaction_response(
            "tx-1",
            &serde_json::json!({"id": "tx-1", "signature": secret}),
        )
        .unwrap_err();
        assert!(transaction.contains("body redacted"));
        assert!(!transaction.contains(secret));

        let wrong_id = parse_relayer_transaction_response(
            "tx-expected",
            &serde_json::json!({
                "id": "tx-other",
                "state": "STATE_CONFIRMED",
                "signature": secret
            }),
        )
        .unwrap_err();
        assert!(wrong_id.contains("did not match"));
        assert!(!wrong_id.contains(secret));
        let missing_id = parse_relayer_transaction_response(
            "tx-expected",
            &serde_json::json!({"state": "STATE_CONFIRMED"}),
        )
        .unwrap_err();
        assert!(missing_id.contains("missing id"));

        let malformed_success = relayer_submit_failure(RelayerHttpError {
            status: 200,
            body: "successful response missing transaction id (body redacted)".into(),
            ambiguous: true,
        });
        assert!(malformed_success.ambiguous);
    }

    #[test]
    fn clob_post_success_requires_an_order_id() {
        assert!(!clob_http_status_is_ambiguous(400));
        assert!(!clob_http_status_is_ambiguous(429));
        assert!(clob_http_status_is_ambiguous(500));
        assert!(clob_http_status_is_ambiguous(504));
        let missing = classify_clob_post_success(serde_json::Value::Null).unwrap_err();
        assert!(missing.ambiguous);
        assert_eq!(missing.status, Some(200));
        assert!(
            classify_clob_post_success(serde_json::json!({
                "orderID": "order-1",
                "status": "live"
            }))
            .is_ok()
        );
    }

    #[test]
    fn approval_required_projects_redacted_challenge_without_grant_or_prf() {
        let prepared = PreparedSigning::ClobAuth(PreparedClobAuth {
            owner: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            nonce: CLOB_AUTH_NONCE,
            timestamp: 1_762_000_001,
            credential_action: "mint_or_derive".into(),
            chain_id: POLYGON,
            signing_hash: format!("{:#x}", B256::repeat_byte(7)),
            review_intent_hash: "review-digest".into(),
        });
        let artifact = approval_artifact_for(
            &prepared,
            "Sealed Approval required for v2 petal sign_hash; action_id=action-123; ceremony_url=http://127.0.0.1:8787/ceremony/token",
        )
        .unwrap()
        .expect("approval challenge");

        assert_eq!(artifact.action_id, "action-123");
        assert_eq!(
            artifact.ceremony_url,
            "http://127.0.0.1:8787/ceremony/token"
        );
        assert_eq!(artifact.expires_ms, None);
        assert_eq!(artifact.retry_state, "approval_required");
        assert_eq!(
            artifact.prepared_artifact_digest,
            prepared_digest(&prepared).unwrap()
        );
        let body = serde_json::to_string(&artifact)
            .unwrap()
            .to_ascii_lowercase();
        assert!(!body.contains("grant"));
        assert!(!body.contains("prf"));
        assert!(!body.contains("signature"));
    }

    #[test]
    fn approval_action_rotation_is_allowed_only_after_expiry() {
        let digest = "prepared-digest";
        let operation = "clob_order";
        let mut artifact = ApprovalArtifact {
            action_id: "original-action".into(),
            ceremony_url: "http://127.0.0.1/ceremony".into(),
            expires_ms: Some(u64::MAX),
            prepared_artifact_digest: digest.into(),
            retry_state: "approval_required".into(),
            operation: operation.into(),
        };
        assert!(
            approval_artifact_matches_at(
                &artifact,
                digest,
                operation,
                Some("replacement-action"),
                100,
            )
            .is_err()
        );

        artifact.expires_ms = Some(0);
        assert!(
            approval_artifact_matches_at(
                &artifact,
                digest,
                operation,
                Some("replacement-action"),
                100,
            )
            .is_ok()
        );
        assert!(approval_artifact_matches_at(&artifact, "changed", operation, None, 100).is_err());
    }

    #[test]
    fn mocked_host_approval_retry_signs_identical_prepared_bytes_and_rejects_mutation() {
        use alloy::signers::{SignerSync, local::PrivateKeySigner};
        use std::str::FromStr;

        let signer = PrivateKeySigner::from_str(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )
        .unwrap();
        let prepared = PreparedSigning::ClobAuth(PreparedClobAuth {
            owner: signer.address().to_checksum(None),
            nonce: CLOB_AUTH_NONCE,
            timestamp: 1_762_000_001,
            credential_action: "mint_or_derive".into(),
            chain_id: POLYGON,
            signing_hash: format!("{:#x}", B256::repeat_byte(7)),
            review_intent_hash: "review-digest".into(),
        });
        let mut mutated = prepared.clone();
        let PreparedSigning::ClobAuth(auth) = &mut mutated else {
            unreachable!();
        };
        auth.timestamp += 1;
        auth.signing_hash = format!("{:#x}", B256::repeat_byte(8));
        let signature = signer
            .sign_hash_sync(&prepared.signing_hash().unwrap())
            .unwrap()
            .as_bytes()
            .to_vec();
        let mutated_signature = signer
            .sign_hash_sync(&mutated.signing_hash().unwrap())
            .unwrap()
            .as_bytes()
            .to_vec();
        bloom_petal_sdk::test_host_reset(vec![
            Ok(SignHashOutcome::ApprovalRequired {
                action_id: "action-1".into(),
                ceremony_url: "http://127.0.0.1/ceremony/action-1".into(),
                expires_ms: u64::MAX,
            }),
            Ok(SignHashOutcome::Signature(signature.clone())),
            Ok(SignHashOutcome::Signature(mutated_signature)),
        ]);
        let approval_key = "actions/alice/test/approval.json";

        let first = sign_prepared("alice", &prepared, approval_key).unwrap_err();
        assert!(matches!(first, DispatchResponse::Error { code: -2, .. }));
        let artifact: ApprovalArtifact = serde_json::from_slice(
            &bloom_petal_sdk::store_get(approval_key, MAX_STORE_BYTES).unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.action_id, "action-1");
        assert_eq!(
            artifact.prepared_artifact_digest,
            prepared_digest(&prepared).unwrap()
        );

        assert_eq!(
            sign_prepared("alice", &prepared, approval_key).unwrap(),
            signature
        );
        assert!(sign_prepared("alice", &mutated, approval_key).is_err());
    }

    #[test]
    fn policy_warnings_require_an_explicit_risk_acknowledgement() {
        assert!(!trade_post_policy_acknowledged(
            &serde_json::json!({"policy_warn": true}),
            false
        ));
        assert!(trade_post_policy_acknowledged(
            &serde_json::json!({"policy_warn": true}),
            true
        ));
        assert!(trade_post_policy_acknowledged(
            &serde_json::json!({"policy_warn": false}),
            false
        ));
    }

    #[test]
    fn direct_pusd_funding_calldata_is_canonical_erc20_transfer() {
        let recipient: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let calldata = erc20_transfer_calldata(recipient, U256::from(25_000_000u64));
        assert_eq!(&calldata[..10], "0xa9059cbb");
        assert_eq!(calldata.len(), 2 + 4 * 2 + 32 * 2 * 2);
        assert!(
            calldata.ends_with("00000000000000000000000000000000000000000000000000000000017d7840")
        );
    }

    #[test]
    fn funding_confirmation_requires_explicit_acknowledgement() {
        assert!(confirmation_body(b"confirm"));
        assert!(confirmation_body(br#"{"confirm":true}"#));
        assert!(!confirmation_body(b""));
        assert!(!confirmation_body(br#"{"confirm":false}"#));
    }

    #[test]
    fn funding_integer_parsing_and_sizing_are_bounded() {
        assert!(positive_decimal("0.000000000000000001"));
        assert!(!positive_decimal("0"));
        assert!(!positive_decimal("1.2.3"));
        assert_eq!(
            parse_decimal_units("1.25", 6).unwrap(),
            U256::from(1_250_000u64)
        );
        assert!(parse_decimal_units("1.0000001", 6).is_err());
        assert!(parse_decimal_units("-1", 6).is_err());
        assert_eq!(
            funding_required_input(
                U256::from(10_000u64),
                U256::from(4_000u64),
                U256::from(5_000u64)
            ),
            U256::from(8_160u64)
        );
        assert_eq!(
            funding_required_input(U256::from(10u64), U256::ONE, U256::ZERO),
            U256::from(10u64)
        );
    }

    #[test]
    fn funding_prepared_digest_binds_review_and_every_transaction() {
        let prepared = PreparedFunding {
            review_intent: serde_json::json!({
                "recipient": "0x1111111111111111111111111111111111111111",
                "max_spend": "1000",
                "slippage_bps": 50,
                "quote_response_digest": "quote"
            }),
            transactions: vec![PreparedEvmTransaction {
                purpose: "enso_swap".into(),
                to: "0x2222222222222222222222222222222222222222".into(),
                value_wei: "1".into(),
                data_hex: "0x00".into(),
            }],
        };
        let original = prepared.digest();
        let mut changed = prepared.clone();
        changed.review_intent["recipient"] =
            serde_json::Value::String("0x3333333333333333333333333333333333333333".into());
        assert_ne!(changed.digest(), original);
        let mut changed = prepared.clone();
        changed.transactions[0].data_hex = "0x01".into();
        assert_ne!(changed.digest(), original);
    }

    #[test]
    fn mocked_outbox_funding_approval_retry_stages_exactly_once() {
        bloom_petal_sdk::test_host_reset(Vec::new());
        let prepared = PreparedFunding {
            review_intent: serde_json::json!({
                "recipient": "0x1111111111111111111111111111111111111111",
                "max_spend": "1",
                "quote_response_digest": "direct-pusd"
            }),
            transactions: vec![PreparedEvmTransaction {
                purpose: "direct_pusd_transfer".into(),
                to: PUSD.to_checksum(None),
                value_wei: "0".into(),
                data_hex: "0xa9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001".into(),
            }],
        };
        let session = StoreFundSession {
            id: "0001".into(),
            wallet: "alice".into(),
            target_pusd: "1".into(),
            max_spend: "1".into(),
            from_token: "pusd".into(),
            slippage_bps: 50,
            deposit_wallet: "0x1111111111111111111111111111111111111111".into(),
            deposit_wallet_source: "factory".into(),
            status: "prepared".into(),
            prepared_funding: Some(prepared),
            review_intent: None,
            outbox_ids: Vec::new(),
            outbox_inspections: Vec::new(),
            next_transaction: 0,
            plan_md: None,
            approval: None,
        };
        assert!(matches!(
            store_put_json("fund/alice/requests/0001.json", &session, false),
            DispatchResponse::Write
        ));
        bloom_petal_sdk::test_host_set_tx_outcomes(
            vec![Ok(bloom_petal_sdk::StagedTransaction {
                outbox_id: "outbox-1".into(),
                plan_md: "exact transaction".into(),
                approval: None,
            })],
            vec![
                Ok(bloom_petal_sdk::StagedTransaction {
                    outbox_id: "outbox-1".into(),
                    plan_md: "approval required".into(),
                    approval: Some(bloom_petal_sdk::OutboxApproval {
                        action_id: "action-1".into(),
                        ceremony_url: "http://127.0.0.1/ceremony/action-1".into(),
                        expires_ms: 1_000,
                    }),
                }),
                Ok(bloom_petal_sdk::StagedTransaction {
                    outbox_id: "outbox-1".into(),
                    plan_md: "broadcast".into(),
                    approval: None,
                }),
            ],
            vec![Ok(bloom_petal_sdk::OutboxInspection {
                outbox_id: "outbox-1".into(),
                state: "pending".into(),
                tx_hash: None,
                receipt_json: None,
            })],
        );

        assert!(matches!(
            write_fund_confirm("alice", "0001", b"confirm"),
            DispatchResponse::Write
        ));
        let first: StoreFundSession = serde_json::from_slice(
            &bloom_petal_sdk::store_get("fund/alice/requests/0001.json", MAX_STORE_BYTES).unwrap(),
        )
        .unwrap();
        assert_eq!(first.outbox_ids, ["outbox-1"]);
        assert_eq!(first.approval.unwrap().action_id, "action-1");

        assert!(matches!(
            write_fund_confirm("alice", "0001", b"confirm"),
            DispatchResponse::Write
        ));
        let second: StoreFundSession = serde_json::from_slice(
            &bloom_petal_sdk::store_get("fund/alice/requests/0001.json", MAX_STORE_BYTES).unwrap(),
        )
        .unwrap();
        assert_eq!(second.outbox_ids, ["outbox-1"]);
        assert!(second.approval.is_none());
        assert_eq!(bloom_petal_sdk::test_host_tx_call_counts(), (1, 2, 1));
    }

    #[test]
    fn funding_calldata_validation_and_exact_approval_are_canonical() {
        let spender: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let approval = erc20_approve_calldata(spender, U256::from(7u64));
        assert!(approval.starts_with("0x095ea7b3"));
        assert_eq!(
            canonical_hex_bytes(&approval, "approval").unwrap().len(),
            68
        );
        assert!(canonical_hex_bytes("0xABC0", "uppercase").is_err());
        assert!(canonical_hex_bytes("0x0", "odd").is_err());
        assert_eq!(
            parse_chain_quantity(r#""0x2a""#, "quantity").unwrap(),
            U256::from(42u64)
        );
    }

    #[test]
    fn relayer_progress_never_persists_a_signature() {
        let progress = RelayerActionProgress {
            prepared_artifact_digest: "digest".into(),
            phase: "submission_started".into(),
            transaction_id: None,
            relayer_state: None,
            transaction_hash: None,
        };
        let encoded = serde_json::to_string(&progress)
            .unwrap()
            .to_ascii_lowercase();
        assert!(!encoded.contains("signature"));
        assert!(!encoded.contains("grant"));
        assert!(!encoded.contains("prf"));
    }

    #[test]
    fn relayer_poll_identity_preserves_and_enforces_submitted_hash() {
        let expected = LocalRelayerTx {
            id: "tx-1".into(),
            state: "STATE_NEW".into(),
            transaction_hash: Some("0xaaa".into()),
        };
        let mut without_hash = LocalRelayerTx {
            id: "tx-1".into(),
            state: "STATE_PENDING".into(),
            transaction_hash: None,
        };
        bind_relayer_transaction_identity(&expected, &mut without_hash).unwrap();
        assert_eq!(without_hash.transaction_hash.as_deref(), Some("0xaaa"));

        let mut changed_hash = LocalRelayerTx {
            id: "tx-1".into(),
            state: "STATE_PENDING".into(),
            transaction_hash: Some("0xbbb".into()),
        };
        assert!(bind_relayer_transaction_identity(&expected, &mut changed_hash).is_err());
        let mut changed_id = LocalRelayerTx {
            id: "tx-2".into(),
            state: "STATE_PENDING".into(),
            transaction_hash: Some("0xaaa".into()),
        };
        assert!(bind_relayer_transaction_identity(&expected, &mut changed_id).is_err());
    }

    #[test]
    fn relayer_receipts_are_terminal_only_for_the_exact_request() {
        let owner: Address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
            .parse()
            .unwrap();
        let recipient: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let calls = vec![PreparedCall::from_call(&transfer_amount_call(
            PUSD,
            recipient,
            U256::from(7u64),
        ))];
        let digest = relayer_request_digest("withdraw-pusd", owner, owner, &calls).unwrap();
        let changed = relayer_request_digest(
            "withdraw-pusd",
            owner,
            owner,
            &[PreparedCall::from_call(&transfer_amount_call(
                PUSD,
                recipient,
                U256::from(8u64),
            ))],
        )
        .unwrap();
        assert_ne!(digest, changed);

        bloom_petal_sdk::test_host_reset(Vec::new());
        let key = "actions/alice/withdraw-pusd/receipt.json";
        assert!(!relayer_receipt_matches(key, &digest).unwrap());
        bloom_petal_sdk::store_put(
            key,
            serde_json::to_string(&serde_json::json!({
                "status": "STATE_CONFIRMED",
                "request_digest": digest,
                "request_marker": "withdraw_all",
            }))
            .unwrap()
            .as_bytes(),
            false,
        )
        .unwrap();
        assert!(relayer_receipt_matches(key, &digest).unwrap());
        assert!(!relayer_receipt_matches(key, &changed).unwrap());
        assert!(
            relayer_action_receipt_matches(
                "alice",
                "withdraw-pusd",
                owner,
                owner,
                &[transfer_amount_call(PUSD, recipient, U256::from(7u64))],
            )
            .unwrap()
        );
        assert!(relayer_terminal_receipt_exists(key).unwrap());
        assert!(relayer_receipt_has_marker(key, "withdraw_all").unwrap());
        assert!(!relayer_receipt_has_marker(key, "other").unwrap());
    }

    #[test]
    fn wasm_relayer_paths_do_not_use_blocking_sleep() {
        assert!(!include_str!("lib.rs").contains(concat!("thread", "::", "sleep")));
    }

    #[test]
    fn withdrawal_amount_is_bounded_and_requires_explicit_confirmation() {
        assert_eq!(
            withdraw_amount(br#"{"confirm":true,"amount":"all"}"#).unwrap(),
            None
        );
        assert_eq!(
            withdraw_amount(br#"{"confirm":true,"amount":"1.25"}"#).unwrap(),
            Some(U256::from(1_250_000u64))
        );
        assert!(withdraw_amount(br#"{"confirm":false,"amount":"1"}"#).is_err());
        assert!(withdraw_amount(br#"{"confirm":true,"amount":"0"}"#).is_ok());
    }
}
