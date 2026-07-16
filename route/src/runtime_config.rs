//! Read-only, daemon-owned runtime configuration for this Petal.

const DEFAULT_CHAIN: &str = "polygon";
const DEFAULT_CHAIN_ID: u64 = 137;

fn setting(key: &str) -> Result<Option<String>, String> {
    petal::sdk::runtime_setting(key).map_err(|err| err.message().to_string())
}

fn endpoint(binding: &str, default: &str) -> String {
    setting(&format!("endpoint.{binding}"))
        .ok()
        .flatten()
        .unwrap_or_else(|| default.to_string())
}

pub fn gamma_url() -> String {
    endpoint("gamma", crate::constants::GAMMA)
}

pub fn data_url() -> String {
    endpoint("data", crate::constants::DATA)
}

pub fn clob_url() -> String {
    endpoint("clob", crate::constants::CLOB)
}

pub fn relayer_url() -> String {
    endpoint("relayer", crate::constants::RELAYER)
}

/// Resolve and verify the configured Bloom chain route against its live chain
/// id before it is used for signing or mediated chain reads.
pub fn configured_chain() -> Result<(String, u64), String> {
    let configured_name = setting("chain")?;
    let configured_id = setting("chain_id")?;
    let (name, id) = match (configured_name, configured_id) {
        (None, None) => (DEFAULT_CHAIN.to_string(), DEFAULT_CHAIN_ID),
        (Some(name), Some(id)) => {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err("configured chain name is invalid".into());
            }
            let id = id
                .parse::<u64>()
                .map_err(|_| "configured chain_id is not a u64".to_string())?;
            (name, id)
        }
        _ => return Err("chain and chain_id must be configured together".into()),
    };
    require_supported_chain_id(id)?;
    Ok((name, id))
}

fn require_supported_chain_id(id: u64) -> Result<(), String> {
    if id != DEFAULT_CHAIN_ID {
        return Err(format!(
            "unsupported Polymarket chain_id {id}; this Petal supports Polygon mainnet ({DEFAULT_CHAIN_ID}) only"
        ));
    }
    Ok(())
}

pub fn chain() -> Result<(String, u64), String> {
    let (name, id) = configured_chain()?;
    let path = format!("chains/{name}/chain_id");
    let live = petal::sdk::vfs_read(&path, 128)
        .map_err(|err| format!("read {path}: {}", err.message()))?;
    let live = std::str::from_utf8(&live)
        .map_err(|_| format!("{path} is not UTF-8"))?
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{path} is not a decimal chain id"))?;
    if live != id {
        return Err(format!(
            "configured chain_id {id} does not match {path} ({live})"
        ));
    }
    Ok((name, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_polygon_mainnet_chain_id_is_supported() {
        assert!(require_supported_chain_id(137).is_ok());
        let err = require_supported_chain_id(80002).unwrap_err();
        assert!(err.contains("supports Polygon mainnet (137) only"));
    }
}
