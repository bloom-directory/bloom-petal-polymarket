use serde::{Deserialize, Serialize};

use crate::polymarket::order::OrderType;
use crate::polymarket::types::Side;

fn default_true() -> bool {
    true
}

/// Lifecycle of a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    Draft,
    Signed,
    Posted,
    Rejected,
    Cancelled,
    /// POST left the box but the outcome is unknown and reconciliation via
    /// open orders failed. Do not retry; investigate.
    Ambiguous,
}

impl DraftStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DraftStatus::Draft => "draft",
            DraftStatus::Signed => "signed",
            DraftStatus::Posted => "posted",
            DraftStatus::Rejected => "rejected",
            DraftStatus::Cancelled => "cancelled",
            DraftStatus::Ambiguous => "ambiguous",
        }
    }
}

/// A reviewable order draft. No secrets, no signatures; safe to expose
/// read-only through the VFS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDraft {
    pub id: String,
    pub wallet: String,
    /// Owner EOA (the key that signs), checksummed.
    pub owner: String,
    /// Funder/maker address: the deposit wallet for signatureType 3, the
    /// owner EOA for legacy signatureType 0. `None` in pre-deposit drafts.
    #[serde(default)]
    pub funder: Option<String>,
    /// CLOB signature type (0 = EOA legacy, 3 = POLY_1271 deposit wallet).
    #[serde(default)]
    pub signature_type: u8,
    pub slug: String,
    pub question: String,
    pub condition_id: String,
    /// YES / NO.
    pub outcome: String,
    pub token_id: String,
    pub side: Side,
    pub order_type: OrderType,
    /// USD leg in micro-USD (spend for buys, proceeds for sells).
    pub amount_microusd: u64,
    /// User bound: max price for buys, min price for sells (micro).
    pub price_bound_micro: u64,
    /// Whether the limit re-derives from the book at confirm (marketable) or
    /// is pinned to `limit_price_micro` (explicit `--limit-price`).
    pub marketable: bool,
    pub limit_price_micro: u64,
    /// Shares in micro-units (multiple of 0.01 shares).
    pub size_micro: u64,
    pub tick_micro: u64,
    pub min_order_size_micro: u64,
    pub neg_risk: bool,
    pub active: bool,
    pub closed: bool,
    pub order_book_enabled: bool,
    /// Whether the market is a true binary YES/NO market (exactly two outcomes).
    /// Validated structurally at snapshot time; carried so confirm-time policy
    /// evaluates the real value instead of assuming binarity. `#[serde(default)]`
    /// makes legacy drafts load as binary, matching prior behavior.
    #[serde(default = "default_true")]
    pub binary_outcomes: bool,
    pub best_ask_micro: Option<u64>,
    pub best_bid_micro: Option<u64>,
    pub book_snapshot_ms: u128,
    /// Snapshot of policy checks at draft time: display only; confirm
    /// re-evaluates policy from current config and receipts.
    pub policy_checks: serde_json::Value,
    pub status: DraftStatus,
    /// Salt of the built order, persisted just before signing so a lost POST
    /// can be reconciled.
    pub salt: Option<u64>,
    pub clob_order_id: Option<String>,
    pub clob_status: Option<String>,
    pub last_error: Option<String>,
    /// Short hash of the passkey review intent presented at confirm.
    #[serde(default)]
    pub review_intent_hash: Option<String>,
    pub created_ms: u128,
    pub updated_ms: u128,
}

/// Durable record of one posted or attempted order. Inputs to the daily policy
/// cap; also what sell uses to reason about holdings history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderReceipt {
    pub draft_id: String,
    pub wallet: String,
    pub slug: String,
    pub token_id: String,
    pub side: Side,
    pub order_type: OrderType,
    /// Funder/maker the order was posted for (deposit wallet on sigtype 3).
    #[serde(default)]
    pub funder: Option<String>,
    /// CLOB signature type the order carried.
    #[serde(default)]
    pub signature_type: u8,
    /// USD leg, micro-USD: requested buy budget or realized sell proceeds.
    pub amount_microusd: u64,
    pub limit_price_micro: u64,
    pub size_micro: u64,
    pub salt: u64,
    pub clob_order_id: Option<String>,
    /// Exact CLOB status string, or `ambiguous` when the response was lost.
    pub clob_status: String,
    /// Filled share size in micro-units when the response reports it.
    pub filled_size_micro: Option<u64>,
    /// Raw CLOB response for audit/debugging.
    pub raw_response: serde_json::Value,
    /// Short hash of the passkey review intent that authorized this order.
    #[serde(default)]
    pub review_intent_hash: Option<String>,
    pub posted_ms: u128,
}
