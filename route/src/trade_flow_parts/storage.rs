use crate::*;

use crate::polymarket::{Result, validate_wallet_name};

pub(crate) fn load_trade_draft(
    wallet: &str,
    id: &str,
) -> Result<StoreTradeDraft, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let Some(bytes) = store_get(&format!("trade/{wallet}/drafts/{id}/order.json")) else {
        return Err(error(-1, "draft not found"));
    };
    let draft: StoreTradeDraft =
        serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt draft: {e}")))?;
    if draft.wallet != wallet || draft.id != id {
        return Err(error(-4, "draft identity mismatch"));
    }
    Ok(draft)
}
