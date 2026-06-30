use crate::*;

use crate::bloom_petal_sdk::DispatchResponse;
use serde::Serialize;
pub(crate) fn read_json_value<T: Serialize>(value: &T) -> DispatchResponse {
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(e) => error(-4, format!("json: {e}")),
    }
}
