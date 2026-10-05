//! Default builder attribution code for order revenue-share.
//!
//! Polymarket's CTF Exchange order struct carries a `bytes32 builder`
//! attribution field (see
//! [`crate::polymarket::order::OrderParams::builder_code`]). It is **not**
//! an address: a builder registers a profile at
//! `polymarket.com/settings?tab=builder`, sets taker/maker fee rates there,
//! and is assigned an opaque `bytes32` code to attach to every order. Fee
//! rates and revenue payout are entirely managed by Polymarket off the back
//! of that registration — this Petal only needs to know which code to
//! attach; there is no on-chain approval step (unlike Hyperliquid's builder
//! fee, which is capped and approved on-chain per builder).
//!
//! A release may declare Bloom's own code so orders carry it without an
//! operator having to configure one. The code is not a secret — it is
//! serialized on-chain in every `OrderFilled` event and builder profiles are
//! publicly queryable — so it is declared in source rather than injected at
//! build time: a release then rebuilds byte for byte from its tag, and the
//! package CI checks is the package that ships. Status therefore reports only
//! whether a default is present, and if so, whether it came from this
//! release or an operator override that can change it without a new release.
use alloy::primitives::B256;
use serde::Serialize;

/// This release's default builder attribution code, used when no operator
/// override is stored. It is public on-chain data, not a credential, so it
/// lives here in source instead of arriving through a build-time environment
/// variable. Set it as hex, with or without `0x`, up to 32 bytes;
/// `release_default_if_set_is_a_valid_builder_code` checks it.
pub const RELEASE_DEFAULT_BUILDER_CODE: Option<&str> = None;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuilderCodeSource {
    StoreOverride,
    ReleaseDefault,
    Unconfigured,
}

#[derive(Debug, Serialize)]
pub struct BuilderCodeStatus {
    pub configured: bool,
    pub source: BuilderCodeSource,
    pub code: Option<String>,
}

/// Resolves the default builder code: an operator-set store override first,
/// so the default can change without a release, then this release's
/// source-declared default, else none.
fn resolve_default(
    release_default: Option<&str>,
    store_override: Option<&str>,
) -> Option<(String, BuilderCodeSource)> {
    if let Some(code) = store_override {
        return Some((code.to_owned(), BuilderCodeSource::StoreOverride));
    }
    release_default.map(|code| (code.to_owned(), BuilderCodeSource::ReleaseDefault))
}

/// The code every order should attach as its builder attribution field: the
/// operator store override if set, else this release's declared default,
/// else none (orders then carry a zero builder field, same as today).
pub fn resolve_default_builder_code(store_override: Option<&str>) -> Option<String> {
    resolve_default(RELEASE_DEFAULT_BUILDER_CODE, store_override).map(|(code, _)| code)
}

pub fn builder_code_status(store_override: Option<&str>) -> BuilderCodeStatus {
    match resolve_default(RELEASE_DEFAULT_BUILDER_CODE, store_override) {
        Some((code, source)) => BuilderCodeStatus {
            configured: true,
            source,
            code: Some(code),
        },
        None => BuilderCodeStatus {
            configured: false,
            source: BuilderCodeSource::Unconfigured,
            code: None,
        },
    }
}

/// Parses a builder code from hex (with or without a `0x` prefix), accepting
/// up to 32 bytes and left-padding a shorter value (e.g. a registered code
/// that happens to be address-shaped) to fill the `bytes32` field, matching
/// Solidity's zero-extension of a narrower value into a wider one.
pub fn parse_builder_code(text: &str) -> Result<B256, String> {
    let hex_str = text.strip_prefix("0x").unwrap_or(text);
    if hex_str.is_empty() || hex_str.len() > 64 {
        return Err("builder code must be 1-64 hex characters (up to 32 bytes)".into());
    }
    if !hex_str.len().is_multiple_of(2) {
        return Err("builder code must have an even number of hex digits".into());
    }
    let bytes = hex::decode(hex_str).map_err(|err| format!("invalid hex: {err}"))?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err("builder code cannot be all zero".into());
    }
    let mut word = [0u8; 32];
    word[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(B256::from(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_override_wins_over_the_release_default() {
        assert_eq!(
            resolve_default(Some("0x02"), Some("0x03")),
            Some(("0x03".into(), BuilderCodeSource::StoreOverride))
        );
    }

    #[test]
    fn release_default_used_when_no_override_is_stored() {
        assert_eq!(
            resolve_default(Some("0x02"), None),
            Some(("0x02".into(), BuilderCodeSource::ReleaseDefault))
        );
    }

    #[test]
    fn nothing_configured_resolves_to_none() {
        assert_eq!(resolve_default(None, None), None);
    }

    #[test]
    fn release_default_if_set_is_a_valid_builder_code() {
        // The default is public data declared in source, so it gets the same
        // check the operator override gets at its write route: an unparsable
        // or all-zero value would otherwise silently attach nothing, or make
        // every order fail once a release declared it.
        if let Some(code) = RELEASE_DEFAULT_BUILDER_CODE {
            parse_builder_code(code).expect("release default builder code must parse");
        }
    }

    #[test]
    fn the_release_wrapper_resolves_exactly_this_release_default() {
        // Whatever this release declares (including nothing), the wrapper
        // resolves exactly that when the store is silent, and a store
        // override still wins over it. Declaring a default breaks no test.
        assert_eq!(
            resolve_default_builder_code(None),
            RELEASE_DEFAULT_BUILDER_CODE.map(str::to_owned)
        );
        assert_eq!(
            resolve_default_builder_code(Some("0x03")),
            Some("0x03".into())
        );
    }

    #[test]
    fn status_reports_the_resolved_source_and_code() {
        let unconfigured = builder_code_status(None);
        assert!(!unconfigured.configured);
        assert_eq!(unconfigured.source, BuilderCodeSource::Unconfigured);
        assert_eq!(unconfigured.code, None);

        let overridden = builder_code_status(Some("0x03"));
        assert!(overridden.configured);
        assert_eq!(overridden.source, BuilderCodeSource::StoreOverride);
        assert_eq!(overridden.code, Some("0x03".into()));
    }

    #[test]
    fn parses_a_full_32_byte_code() {
        let code = "0x000000000000000000000000000000000000000000000000000000000000dead";
        let parsed = parse_builder_code(code).unwrap();
        assert_eq!(parsed.0[30..], [0xde, 0xad]);
    }

    #[test]
    fn left_pads_a_shorter_address_shaped_code() {
        let parsed = parse_builder_code("0x000000000000000000000000000000000000dEaD").unwrap();
        assert_eq!(&parsed.0[..12], &[0u8; 12]);
        assert_eq!(
            &parsed.0[12..],
            hex::decode("000000000000000000000000000000000000dead")
                .unwrap()
                .as_slice()
        );
    }

    #[test]
    fn rejects_all_zero_code() {
        assert!(parse_builder_code("0x00").is_err());
        assert!(parse_builder_code(&"0".repeat(64)).is_err());
    }

    #[test]
    fn rejects_odd_length_and_oversized_input() {
        assert!(parse_builder_code("0xabc").is_err());
        assert!(parse_builder_code(&format!("0x{}", "ab".repeat(33))).is_err());
    }
}
