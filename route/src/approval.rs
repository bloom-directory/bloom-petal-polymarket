use crate::prelude::*;
use alloy::primitives::{Address, B256, Signature};
use petal::sdk::DispatchResponse;

pub fn store_review_intent(
    key: &str,
    review_intent: &serde_json::Value,
) -> Result<String, DispatchResponse> {
    let bytes = serde_json::to_vec(review_intent)
        .map_err(|err| error(-4, format!("encode review intent: {err}")))?;
    petal::sdk::store_put(key, &bytes, false).map_err(sdk_error)?;
    Ok(blake3_hex(&bytes))
}

pub fn verify_review_intent(key: &str, expected_hash: &str) -> Result<(), DispatchResponse> {
    let bytes = petal::sdk::store_get(key, MAX_STORE_BYTES)
        .map_err(|error| sdk_error_with_context("read review intent", error))?;
    if blake3_hex(&bytes) != expected_hash {
        return Err(error(
            -4,
            "review intent does not match the prepared operation",
        ));
    }
    Ok(())
}

/// Validate the returned signature and render v as 27/28 for the venue.
pub fn verified_signature(
    owner: Address,
    hash: B256,
    bytes: &[u8],
) -> Result<Signature, DispatchResponse> {
    let signature =
        Signature::from_raw(bytes).map_err(|err| error(-4, format!("host signature: {err}")))?;
    if signature
        .recover_address_from_prehash(&hash)
        .map_err(|err| error(-4, format!("recover host signature: {err}")))?
        != owner
    {
        return Err(error(-4, "host signature does not match prepared owner"));
    }
    Ok(signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_owner_and_normalizes_signature() {
        use alloy::signers::{SignerSync, local::PrivateKeySigner};
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(7)).unwrap();
        let hash = alloy::primitives::keccak256([1, 2, 3]);
        let signature = signer.sign_hash_sync(&hash).unwrap();
        let mut bytes = signature.as_bytes();
        bytes[64] -= 27;
        assert_eq!(
            verified_signature(signer.address(), hash, &bytes).unwrap(),
            signature
        );
        assert!(verified_signature(Address::repeat_byte(9), hash, &bytes).is_err());
        assert!(verified_signature(signer.address(), hash, &bytes[..64]).is_err());
    }
}
