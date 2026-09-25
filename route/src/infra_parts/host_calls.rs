use crate::prelude::*;

use crate::polymarket::Result;
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus, HttpRequest, SdkError};

/// Resolve the selected account EVM owner of the Polymarket deposit wallet.
pub fn wallet_address(wallet: &str) -> Result<Address, DispatchResponse> {
    petal::validate_wallet_id(wallet).map_err(|message| error(-3, message))?;
    let path = wallet_address_path(wallet);
    let bytes = petal::sdk::vfs_read(&path, 128).map_err(|e| match e {
        SdkError::Host(HostStatus::Denied) => {
            error(-2, format!("wallet {wallet}: read of {path} was denied"))
        }
        // Bloom's wallet VFS answers a missing account leaf with `NotAFile`,
        // which reaches the Petal as `Invalid`.
        SdkError::Host(HostStatus::NotFound | HostStatus::Invalid) => error(
            -1,
            format!(
                "wallet {wallet} has no EVM address at {path}; the account may be missing or have no EVM key"
            ),
        ),
        other => error(-4, format!("wallet {wallet}: read {path}: {}", other.message())),
    })?;
    parse_wallet_address(&path, &bytes)
        .map_err(|message| error(-4, format!("wallet {wallet}: {message}")))
}

pub(crate) fn wallet_address_path(wallet: &str) -> String {
    format!("wallets/{wallet}/{}/address.evm", crate::account::number())
}

fn parse_wallet_address(path: &str, bytes: &[u8]) -> Result<Address, String> {
    let raw = core::str::from_utf8(bytes)
        .map_err(|_| format!("EVM address at {path} is not UTF-8"))?
        .trim();
    raw.parse::<Address>()
        .map_err(|e| format!("invalid EVM address at {path}: {e}"))
}

pub fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<petal::sdk::HttpResponse, DispatchResponse> {
    petal::sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
            body,
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_address_reads_account_scoped_evm_leaf() {
        assert_eq!(wallet_address_path("main"), "wallets/main/0/address.evm");
    }

    #[test]
    fn wallet_address_parses_trimmed_checksummed_address() {
        let path = wallet_address_path("main");
        let address = parse_wallet_address(&path, b"0x52908400098527886E0F7030069857D2E4169EE7\n")
            .expect("valid address");
        assert_eq!(
            address.to_checksum(None),
            "0x52908400098527886E0F7030069857D2E4169EE7"
        );
    }

    #[test]
    fn wallet_address_errors_name_the_path() {
        let path = wallet_address_path("main");
        let err = parse_wallet_address(&path, b"not-an-address").unwrap_err();
        assert!(err.contains("wallets/main/0/address.evm"), "{err}");
        let err = parse_wallet_address(&path, &[0xff, 0xfe]).unwrap_err();
        assert!(err.contains("wallets/main/0/address.evm"), "{err}");
    }

    #[test]
    fn no_route_source_reads_removed_wallet_paths() {
        fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read source dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    visit(&path, hits);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("infra_parts/host_calls.rs")
                    && !path.ends_with("app_tests.rs")
                {
                    let source = std::fs::read_to_string(&path).expect("read source");
                    for removed in [
                        "wallets/{wallet}/address\"",
                        "/addresses.json",
                        "wallets/{wallet}/public_key",
                        "/policy.toml",
                    ] {
                        if source.contains(removed) {
                            hits.push(format!("{} reads {removed}", path.display()));
                        }
                    }
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut hits = Vec::new();
        visit(&root.join("src"), &mut hits);
        visit(&root.join("files"), &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }
}
