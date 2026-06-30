use crate::*;

use crate::bloom_petal_sdk::DispatchResponse;
use crate::order::format_micro;
use serde::Serialize;
pub(crate) fn read_json_value<T: Serialize>(value: &T) -> DispatchResponse {
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(e) => error(-4, format!("json: {e}")),
    }
}

pub(crate) fn render_onboard_plan(wallet: &str) -> String {
    format!(
        "# Polymarket onboarding\n\nWallet: {wallet}\n\nWrite `begin` to request daemon-keystore signatures for CLOB auth and any required deposit-wallet approval batch, store CLOB and builder credentials in the private petal store, deploy the live-factory deposit wallet when needed, rest at `fund` until pUSD arrives, then approve and sync CLOB buying power before marking the wallet tradeable.\n"
    )
}

pub(crate) fn render_trade_plan(draft: &StoreTradeDraft) -> String {
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

pub(crate) fn render_fund_plan(session: &StoreFundSession) -> String {
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
