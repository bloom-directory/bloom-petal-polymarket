use crate::prelude::*;

use crate::polymarket::{BuilderCredentials, Credentials, Result};
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
pub fn load_creds(wallet: &str) -> Result<Credentials, DispatchResponse> {
    let Some(bytes) = store_get(&format!("creds/{wallet}/clob.json")) else {
        return Err(error(
            -3,
            format!("wallet '{wallet}' is not onboarded; write onboard/{wallet}/begin first"),
        ));
    };
    serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt credentials: {e}")))
}

pub fn load_builder_credentials(
    wallet: &str,
) -> Result<Option<BuilderCredentials>, DispatchResponse> {
    match petal::sdk::store_get(&format!("creds/{wallet}/builder.json"), MAX_STORE_BYTES) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| error(-4, format!("corrupt builder credentials: {e}"))),
        Err(SdkError::Host(HostStatus::NotFound)) => Ok(None),
        Err(e) => Err(sdk_error(e)),
    }
}

pub fn save_builder_credentials(
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

pub fn delete_builder_credentials(wallet: &str) -> Result<(), DispatchResponse> {
    match petal::sdk::store_del(&format!("creds/{wallet}/builder.json")) {
        Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => Ok(()),
        Err(e) => Err(sdk_error(e)),
    }
}

pub fn ensure_builder_credentials(
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
