use crate::polymarket::eip712::{Batch, FACTORY, batch_signing_hash};
use crate::polymarket::wallet::approval_calls;
use crate::polymarket::{BuilderCredentials, Credentials, Result};
use crate::prelude::*;
use alloy::primitives::{Address, B256, U256};
use petal::sdk::{DispatchResponse, HttpRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct PreparedRelayerSignature {
    prepared_digest: String,
    signature_hex: String,
}

pub(crate) fn prepared_relayer_batch_expired(
    prepared: &PreparedSigning,
    now_secs: u64,
) -> Result<bool, DispatchResponse> {
    let deadline = prepared
        .preimage
        .get("deadline")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| error(-4, "prepared onboarding batch is missing deadline"))?;
    Ok(now_secs >= deadline)
}
pub struct LocalRelayerTx {
    pub id: String,
    pub state: String,
}

impl LocalRelayerTx {
    pub fn is_confirmed(&self) -> bool {
        self.state == "STATE_CONFIRMED"
    }

    pub fn is_failed(&self) -> bool {
        let state = self.state.to_ascii_uppercase();
        state.contains("FAIL") || state.contains("INVALID")
    }
}

pub fn relayer_submit_configured(
    wallet: &str,
    owner: Address,
    clob_creds: &Credentials,
    body: serde_json::Value,
) -> Result<LocalRelayerTx, DispatchResponse> {
    match crate::relayer_config::configured_relayer_auth()? {
        crate::relayer_config::RelayerAuth::AutoBuilder => {
            relayer_submit_with_builder_repair(wallet, owner, clob_creds, body)
        }
        crate::relayer_config::RelayerAuth::Manual { key, address } => {
            relayer_submit_with_headers(manual_relayer_headers(&key, &address), &body)
                .map_err(relayer_http_error)
        }
        crate::relayer_config::RelayerAuth::Disabled { reason } => Err(error(-3, reason)),
    }
}

fn relayer_submit_with_builder_repair(
    wallet: &str,
    owner: Address,
    clob_creds: &Credentials,
    body: serde_json::Value,
) -> Result<LocalRelayerTx, DispatchResponse> {
    let mut builder = ensure_builder_credentials(wallet, owner, clob_creds)?;
    match relayer_submit(&builder, &body) {
        Ok(tx) => Ok(tx),
        Err(RelayerHttpError {
            status: 401 | 403, ..
        }) => {
            let listed = clob_l2_get_json(owner, clob_creds, "/auth/builder-api-key", &[])?;
            let entries = listed
                .as_array()
                .or_else(|| listed.get("data").and_then(serde_json::Value::as_array))
                .or_else(|| listed.get("keys").and_then(serde_json::Value::as_array));
            let stored_is_active = entries
                .into_iter()
                .flatten()
                .filter_map(crate::polymarket::builder_creds::BuilderApiKeyInfo::from_value)
                .any(|info| info.key == builder.key && info.revoked_at.is_none());
            if stored_is_active {
                return Err(error(
                    -4,
                    "relayer rejected an active builder key; refusing destructive rotation",
                ));
            }
            delete_builder_credentials(wallet)?;
            builder = ensure_builder_credentials(wallet, owner, clob_creds)?;
            relayer_submit(&builder, &body).map_err(relayer_http_error)
        }
        Err(err) => Err(relayer_http_error(err)),
    }
}

#[derive(Debug)]
pub struct RelayerHttpError {
    status: u16,
    body: String,
}

pub fn relayer_submit(
    builder: &BuilderCredentials,
    body: &serde_json::Value,
) -> Result<LocalRelayerTx, RelayerHttpError> {
    let encoded = serde_json::to_string(body).map_err(|e| RelayerHttpError {
        status: 0,
        body: format!("relayer body JSON: {e}"),
    })?;
    let headers = builder_headers(builder, "POST", "/submit", &encoded).map_err(|message| {
        RelayerHttpError {
            status: 0,
            body: message,
        }
    })?;
    let headers: Vec<(String, String)> = headers
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    relayer_submit_with_headers(headers, body)
}

fn relayer_submit_with_headers(
    mut headers: Vec<(String, String)>,
    body: &serde_json::Value,
) -> Result<LocalRelayerTx, RelayerHttpError> {
    let body = serde_json::to_string(body).map_err(|e| RelayerHttpError {
        status: 0,
        body: format!("relayer body JSON: {e}"),
    })?;
    headers.push(("content-type".into(), "application/json".into()));
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "POST".into(),
            url: format!("{}/submit", crate::runtime_config::relayer_url()),
            headers,
            body: body.into_bytes(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(|e| RelayerHttpError {
        status: 0,
        body: e.message().to_string(),
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(RelayerHttpError {
            status: resp.status,
            body: format!(
                "relayer /submit response body redacted ({} bytes)",
                resp.body.len()
            ),
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| RelayerHttpError {
            status: resp.status,
            body: format!("relayer submit JSON: {e}"),
        })?;
    parse_relayer_submit_response(&value).map_err(|body| RelayerHttpError {
        status: resp.status,
        body,
    })
}

pub fn relayer_wallet_nonce(owner: Address) -> Result<u64, DispatchResponse> {
    let value = relayer_get_json(&url_with_query(
        &format!("{}/nonce", crate::runtime_config::relayer_url()),
        &[("address", &format!("{owner:#x}")), ("type", "WALLET")],
    ))?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .or_else(|| value.get("nonce").and_then(parse_json_u64))
        .ok_or_else(|| error(-4, format!("relayer /nonce response unparsable: {value}")))
}

pub fn relayer_poll_confirmed(tx: &LocalRelayerTx) -> Result<LocalRelayerTx, DispatchResponse> {
    let current = relayer_transaction(&tx.id)?;
    if current.is_confirmed() {
        Ok(current)
    } else if current.is_failed() {
        Err(error(
            -4,
            format!("relayer tx {} {}", current.id, current.state),
        ))
    } else {
        Err(error(
            -2,
            format!(
                "relayer tx {} is {}; retry this write to poll again",
                current.id, current.state
            ),
        ))
    }
}

pub fn relayer_transaction(id: &str) -> Result<LocalRelayerTx, DispatchResponse> {
    let value = relayer_get_json(&url_with_query(
        &format!("{}/transaction", crate::runtime_config::relayer_url()),
        &[("id", id)],
    ))?;
    parse_relayer_transaction_response(id, &value).map_err(|message| error(-4, message))
}

pub fn relayer_get_json(url: &str) -> Result<serde_json::Value, DispatchResponse> {
    let headers = match crate::relayer_config::configured_relayer_auth()? {
        crate::relayer_config::RelayerAuth::AutoBuilder => Vec::new(),
        crate::relayer_config::RelayerAuth::Manual { key, address } => {
            manual_relayer_headers(&key, &address)
        }
        crate::relayer_config::RelayerAuth::Disabled { reason } => return Err(error(-3, reason)),
    };
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: url.into(),
            headers,
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!(
                "relayer error (status {}): {}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ),
        ));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("relayer JSON: {e}")))
}

pub fn manual_relayer_headers(key: &str, address: &str) -> Vec<(String, String)> {
    vec![
        ("RELAYER_API_KEY".into(), key.into()),
        ("RELAYER_API_KEY_ADDRESS".into(), address.into()),
    ]
}

pub fn relayer_batch_body(
    wallet: &str,
    owner: Address,
    deposit: Address,
    nonce: u64,
    deadline: u64,
) -> Result<serde_json::Value, DispatchResponse> {
    let prepared = prepare_relayer_batch(wallet, owner, deposit, nonce, deadline)?;
    let prepared_deadline = prepared
        .preimage
        .get("deadline")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| error(-4, "prepared onboarding batch is missing deadline"))?;
    if prepared_relayer_batch_expired(&prepared, now_secs())? {
        let _ = petal::sdk::store_del(&format!("onboard/{wallet}/prepared_relayer_batch.json"));
        let _ = petal::sdk::store_del(&format!("onboard/{wallet}/prepared_relayer_signature.json"));
        return Err(error(
            -2,
            "prepared onboarding approval signature expired; retry onboarding to prepare and approve a fresh bounded batch",
        ));
    }
    let calls_json = prepared
        .preimage
        .get("calls")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, "prepared onboarding batch is missing calls"))?;
    let prepared_nonce = prepared
        .preimage
        .get("nonce")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| error(-4, "prepared onboarding batch is missing nonce"))?;
    let signature_key = format!("onboard/{wallet}/prepared_relayer_signature.json");
    let signature = match petal::sdk::store_get(&signature_key, MAX_STORE_BYTES) {
        Ok(bytes) => {
            let stored: PreparedRelayerSignature = serde_json::from_slice(&bytes)
                .map_err(|err| error(-4, format!("stored relayer signature: {err}")))?;
            if stored.prepared_digest != prepared.digest()? {
                return Err(error(
                    -4,
                    "stored relayer signature does not match prepared batch",
                ));
            }
            stored.signature_hex
        }
        Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => format!(
            "0x{}",
            hex::encode(sign_prepared(
                wallet,
                &prepared,
                &format!("onboard/{wallet}/approval.json"),
            )?)
        ),
        Err(err) => return Err(sdk_error(err)),
    };
    Ok(serde_json::json!({
        "type": "WALLET",
        "from": owner.to_checksum(None),
        "to": FACTORY.to_checksum(None),
        "nonce": prepared_nonce.to_string(),
        "signature": signature,
        "depositWalletParams": {
            "depositWallet": deposit.to_checksum(None),
            "deadline": prepared_deadline.to_string(),
            "calls": calls_json,
        },
    }))
}

pub fn prepare_relayer_batch(
    wallet: &str,
    owner: Address,
    deposit: Address,
    nonce: u64,
    deadline: u64,
) -> Result<PreparedSigning, DispatchResponse> {
    let (_, chain_id) = crate::runtime_config::chain().map_err(|err| error(-4, err))?;
    let prepared_key = format!("onboard/{wallet}/prepared_relayer_batch.json");
    let review_key = format!("onboard/{wallet}/review_relayer_intent.json");
    let prepared = match load_prepared_signing(&prepared_key)? {
        Some(prepared) => prepared,
        None => {
            let calls = approval_calls();
            let batch = Batch {
                wallet: deposit,
                nonce: U256::from(nonce),
                deadline: U256::from(deadline),
                calls: calls.clone(),
            };
            let hash = batch_signing_hash(&batch, chain_id, deposit);
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
            let review = serde_json::json!({
                "operation": "onboard_approvals",
                "owner": owner.to_checksum(None),
                "deposit_wallet": deposit.to_checksum(None),
                "chain_id": chain_id,
                "nonce": nonce,
                "deadline": deadline,
                "calls": calls_json,
                "signing_hash": format!("{hash:#x}"),
            });
            let review_hash = store_review_intent(&review_key, &review)?;
            let prepared = PreparedSigning::new(
                "onboard_approvals",
                "polymarket.onboard",
                owner,
                hash,
                serde_json::json!({
                    "deposit_wallet": deposit.to_checksum(None),
                    "chain_id": chain_id,
                    "nonce": nonce,
                    "deadline": deadline,
                    "calls": calls_json,
                    "review_intent_hash": review_hash,
                }),
            );
            store_prepared_signing(&prepared_key, &prepared)?;
            prepared
        }
    };
    if prepared.operation != "onboard_approvals" || prepared.owner != owner.to_checksum(None) {
        return Err(error(-4, "prepared onboarding batch identity mismatch"));
    }
    if prepared
        .preimage
        .get("chain_id")
        .and_then(serde_json::Value::as_u64)
        != Some(chain_id)
    {
        return Err(error(
            -4,
            "prepared onboarding batch does not match configured chain",
        ));
    }
    let review_hash = prepared
        .preimage
        .get("review_intent_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "prepared onboarding batch is missing review hash"))?;
    verify_review_intent(&review_key, review_hash)?;
    Ok(prepared)
}

pub fn store_prepared_relayer_signature(
    wallet: &str,
    prepared: &PreparedSigning,
    signature: &[u8],
) -> Result<(), DispatchResponse> {
    let value = PreparedRelayerSignature {
        prepared_digest: prepared.digest()?,
        signature_hex: format!("0x{}", hex::encode(signature)),
    };
    match store_put_json(
        &format!("onboard/{wallet}/prepared_relayer_signature.json"),
        &value,
        true,
    ) {
        DispatchResponse::Write => Ok(()),
        response => Err(response),
    }
}

pub fn sign_hash_hex(wallet: &str, purpose: &str, hash: B256) -> Result<String, DispatchResponse> {
    let owner = wallet_address(wallet)?;
    let prepared = PreparedSigning::new(
        "relayer_batch",
        purpose,
        owner,
        hash,
        serde_json::json!({"signing_hash": format!("{hash:#x}")}),
    );
    sign_prepared(
        wallet,
        &prepared,
        &format!("onboard/{wallet}/approval.json"),
    )
    .map(|signature| format!("0x{}", hex::encode(signature)))
}

pub fn builder_headers(
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

pub fn builder_hmac_signature(
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

pub fn parse_relayer_submit_response(value: &serde_json::Value) -> Result<LocalRelayerTx, String> {
    let id = ["transactionID", "transactionId", "transaction_id", "id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .ok_or_else(|| format!("relayer /submit response missing transaction id: {value}"))?;
    let state = value
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("STATE_NEW");
    Ok(LocalRelayerTx {
        id: id.into(),
        state: state.into(),
    })
}

pub fn parse_relayer_transaction_response(
    id: &str,
    value: &serde_json::Value,
) -> Result<LocalRelayerTx, String> {
    let tx = match value {
        serde_json::Value::Array(items) => items
            .iter()
            .find(|item| relayer_tx_id_matches(item, id))
            .ok_or_else(|| format!("relayer /transaction response did not contain {id}"))?,
        other => other,
    };
    let state = tx
        .get("state")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("relayer /transaction response for {id} missing state: {value}"))?;
    let parsed_id = ["transactionID", "transactionId", "transaction_id", "id"]
        .iter()
        .find_map(|key| tx.get(*key).and_then(serde_json::Value::as_str))
        .unwrap_or(id);
    if parsed_id != id {
        return Err(format!(
            "relayer /transaction returned id {parsed_id} while polling {id}"
        ));
    }
    Ok(LocalRelayerTx {
        id: parsed_id.into(),
        state: state.into(),
    })
}

pub fn relayer_tx_id_matches(value: &serde_json::Value, id: &str) -> bool {
    ["transactionID", "transactionId", "transaction_id", "id"]
        .iter()
        .any(|key| value.get(*key).and_then(serde_json::Value::as_str) == Some(id))
}

pub fn relayer_http_error(err: RelayerHttpError) -> DispatchResponse {
    if err.status == 401 || err.status == 403 {
        return error(
            -4,
            format!(
                "relayer rejected authentication (status {}): {}",
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

pub fn parse_json_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

pub fn onboard_in_flight_deadline_ms() -> u128 {
    now_millis().saturating_add((ONBOARD_POLL_TIMEOUT_SECS as u128).saturating_mul(1000))
}

pub fn dispatch_error_message(resp: &DispatchResponse) -> String {
    match resp {
        DispatchResponse::Error { message, .. } => message.clone(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_poll_requires_exact_id() {
        let response = serde_json::json!([
            {"transactionID": "other", "state": "STATE_CONFIRMED"},
            {"transactionID": "wanted", "state": "STATE_NEW"}
        ]);
        let parsed = parse_relayer_transaction_response("wanted", &response).unwrap();
        assert_eq!(parsed.id, "wanted");
        assert_eq!(parsed.state, "STATE_NEW");
        assert!(parse_relayer_transaction_response("missing", &response).is_err());
        assert!(
            parse_relayer_transaction_response(
                "wanted",
                &serde_json::json!({"transactionID": "other", "state": "STATE_CONFIRMED"}),
            )
            .is_err()
        );
    }

    #[test]
    fn manual_auth_uses_native_header_names() {
        assert_eq!(
            manual_relayer_headers("key", "0xowner"),
            vec![
                ("RELAYER_API_KEY".into(), "key".into()),
                ("RELAYER_API_KEY_ADDRESS".into(), "0xowner".into()),
            ]
        );
    }
}
