use crate::*;

use crate::bloom_petal_sdk::DispatchResponse;
use crate::order::parse_micro;
use crate::polymarket::{Result, validate_wallet_name};
pub(crate) fn create_fund_request(wallet: &str, body: &[u8]) -> DispatchResponse {
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
    if parse_micro(req.max_spend.trim()).unwrap_or(0) == 0 {
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
    };
    store_put_json(
        &format!("fund/{wallet}/requests/{id}.json"),
        &session,
        false,
    )
}

pub(crate) fn load_fund_session(
    wallet: &str,
    id: &str,
) -> Result<StoreFundSession, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let Some(bytes) = store_get(&format!("fund/{wallet}/requests/{id}.json")) else {
        return Err(error(-1, "not found"));
    };
    serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt fund request: {e}")))
}
