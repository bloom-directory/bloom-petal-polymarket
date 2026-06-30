use crate::prelude::*;

use petal::sdk::{DispatchResponse, HttpRequest};
use crate::polymarket::Result;

pub(crate) fn check_geoblock() -> Result<(), DispatchResponse> {
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: format!("{POLYMARKET_WEB}/api/geoblock"),
            headers: Vec::new(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(|e| {
        error(
            -3,
            format!(
                "could not verify region availability (geoblock check failed: {}); refusing",
                e.message()
            ),
        )
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -3,
            format!(
                "could not verify region availability (geoblock status {}); refusing",
                resp.status
            ),
        ));
    }
    let status: GeoblockStatus = serde_json::from_slice(&resp.body).map_err(|e| {
        error(
            -3,
            format!("could not verify region availability (geoblock JSON: {e}); refusing"),
        )
    })?;
    if status.blocked {
        return Err(error(
            -3,
            format!(
                "Polymarket is unavailable in your region (country={}, region={}); refusing to onboard",
                status.country, status.region
            ),
        ));
    }
    Ok(())
}
