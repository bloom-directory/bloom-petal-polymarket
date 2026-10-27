use crate::prelude::*;

use crate::polymarket::Result;
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HttpRequest};

/// Resolve the selected account's EVM owner of the Polymarket deposit wallet,
/// from `wallets/<wallet>/<account>/address.evm` as Bloom projects it.
pub fn wallet_address(account: &petal::WalletAccount) -> Result<Address, DispatchResponse> {
    owner_address(account, &mut petal::HostVfs)
}

fn owner_address<R: petal::VfsRead>(
    account: &petal::WalletAccount,
    vfs: &mut R,
) -> Result<Address, DispatchResponse> {
    let raw = account.address_evm_from(vfs)?;
    raw.parse::<Address>().map_err(|err| {
        error(
            -4,
            format!(
                "invalid EVM address at {}: {err}",
                account.address_evm_path()
            ),
        )
    })
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

    use petal::testing::{MapVfs, wallet_account};
    use petal::{DispatchResponse, HostStatus, SdkError};

    fn code(response: DispatchResponse) -> i32 {
        match response {
            DispatchResponse::Error { code, .. } => code,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn wallet_address_reads_the_selected_account_evm_leaf() {
        let vfs = || {
            MapVfs::new()
                .file(
                    "wallets/main/0/address.evm",
                    b"0x52908400098527886E0F7030069857D2E4169EE7\n",
                )
                .file(
                    "wallets/main/1/address.evm",
                    b"0x8617E340B3D01FA5F11F306F4090FD50E238070D\n",
                )
        };
        let zero = owner_address(&wallet_account("main", 0), &mut vfs()).unwrap();
        let one = owner_address(&wallet_account("main", 1), &mut vfs()).unwrap();
        assert_eq!(
            zero.to_checksum(None),
            "0x52908400098527886E0F7030069857D2E4169EE7"
        );
        assert_ne!(zero, one);
    }

    #[test]
    fn wallet_address_errors_name_the_path() {
        let account = wallet_account("main", 0);
        assert_eq!(
            code(owner_address(&account, &mut MapVfs::new()).unwrap_err()),
            -1
        );
        let denied = MapVfs::new().failing(
            "wallets/main/0/address.evm",
            SdkError::Host(HostStatus::Denied),
        );
        assert_eq!(
            code(owner_address(&account, &mut { denied }).unwrap_err()),
            -2
        );
        let garbage = MapVfs::new().file("wallets/main/0/address.evm", b"not-an-address");
        match owner_address(&account, &mut { garbage }).unwrap_err() {
            DispatchResponse::Error { code, message } => {
                assert_eq!(code, -4);
                assert!(message.contains("wallets/main/0/address.evm"), "{message}");
            }
            other => panic!("{other:?}"),
        }
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
