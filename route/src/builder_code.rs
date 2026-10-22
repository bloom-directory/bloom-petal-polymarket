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
//! A release build embeds Bloom's own code so orders carry it without an
//! operator having to configure one. The code is not a secret — it is
//! serialized on-chain in every `OrderFilled` event and builder profiles are
//! publicly queryable — but it is injected at build time rather than
//! committed, because this repository is public. The cost is that a tagged
//! release cannot be rebuilt byte for byte from its source alone. What
//! compensates is that the code is bound into the approval the owner signs,
//! so an order cannot carry a code the owner did not approve. Status
//! therefore reports only whether a default is present, and if so, whether it
//! came from this release or an operator override that can change it without
//! a new release.
use alloy::primitives::B256;
use serde::{Deserialize, Serialize};

/// This release's default builder attribution code, used when no operator
/// override is stored. A release build embeds it from the
/// `POLYMARKET_BUILDER_CODE` build input; a local build sets nothing, so
/// orders carry a zero builder field as they did before this was wired up.
///
/// Set it as hex, with or without `0x`, up to 32 bytes;
/// `release_default_if_set_is_a_valid_builder_code` checks whatever a build
/// embeds, so an unparsable or all-zero value cannot silently attach nothing.
pub const EMBEDDED_DEFAULT_BUILDER_CODE: Option<&str> = option_env!("POLYMARKET_BUILDER_CODE");

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
    resolve_default(EMBEDDED_DEFAULT_BUILDER_CODE, store_override).map(|(code, _)| code)
}

pub fn builder_code_status(store_override: Option<&str>) -> BuilderCodeStatus {
    match resolve_default(EMBEDDED_DEFAULT_BUILDER_CODE, store_override) {
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

/// Polymarket's ceiling on a builder's registered rates, by side. A profile
/// cannot be configured above these, so a ceiling above them could never be
/// charged and almost certainly means a misconfiguration.
pub const VENUE_MAX_MAKER_FEE_BPS: u16 = 50;
pub const VENUE_MAX_TAKER_FEE_BPS: u16 = 100;

/// The most this Petal will let a builder code charge, by side, in basis
/// points of notional.
///
/// Polymarket serves no API for a builder profile's registered rates, so the
/// Petal cannot read what a code will actually charge. This is the operator's
/// declared ceiling instead: the owner approves it, and an order is sized and
/// bounded against it rather than against a rate nobody here can see. A
/// change to the real profile takes effect three days after it is scheduled,
/// with a seven-day cooldown, so a configured ceiling has warning before it
/// drifts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderFeeCeilingBps {
    pub maker_bps: u16,
    pub taker_bps: u16,
}

impl BuilderFeeCeilingBps {
    /// Rejects a ceiling the venue could never charge. Granularity is 1 bp by
    /// construction, since the fields are whole basis points.
    pub fn validate(self) -> Result<Self, String> {
        // Refusing a configured code with no ceiling, then accepting an
        // all-zero one, would let the same zero-fee claim through the front
        // door. A builder charging nothing does not need a code attached.
        if self.maker_bps == 0 && self.taker_bps == 0 {
            return Err(
                "a ceiling of zero on both sides would declare no fee; clear settings/builder-code instead if the builder charges nothing"
                    .to_owned(),
            );
        }
        if self.maker_bps > VENUE_MAX_MAKER_FEE_BPS {
            return Err(format!(
                "maker_bps must be 0..={VENUE_MAX_MAKER_FEE_BPS}; Polymarket caps a maker rate there"
            ));
        }
        if self.taker_bps > VENUE_MAX_TAKER_FEE_BPS {
            return Err(format!(
                "taker_bps must be 0..={VENUE_MAX_TAKER_FEE_BPS}; Polymarket caps a taker rate there"
            ));
        }
        Ok(self)
    }
}

/// This release's default fee ceiling. Unlike the builder code this is not
/// sensitive and is not injected at build time, so it stays reproducible from
/// source. `None` until an operator sets one, which is why a configured code
/// without a ceiling is refused at post time rather than assumed to be free.
pub const RELEASE_DEFAULT_FEE_CEILING: Option<BuilderFeeCeilingBps> = None;

/// Resolves the fee ceiling: an operator-set store override first, so it can
/// change without a release, then this release's default, else none.
pub fn resolve_fee_ceiling(
    release_default: Option<BuilderFeeCeilingBps>,
    store_override: Option<BuilderFeeCeilingBps>,
) -> Option<(BuilderFeeCeilingBps, BuilderCodeSource)> {
    if let Some(ceiling) = store_override {
        return Some((ceiling, BuilderCodeSource::StoreOverride));
    }
    release_default.map(|ceiling| (ceiling, BuilderCodeSource::ReleaseDefault))
}

/// Convenience wrapper baking in this release's default.
pub fn resolve_default_fee_ceiling(
    store_override: Option<BuilderFeeCeilingBps>,
) -> Option<BuilderFeeCeilingBps> {
    resolve_fee_ceiling(RELEASE_DEFAULT_FEE_CEILING, store_override).map(|(ceiling, _)| ceiling)
}

/// `bps` as a percentage for an owner-facing line: 1 -> "0.01%", 100 -> "1%".
pub fn bps_percent_string(bps: u16) -> String {
    let whole = bps / 100;
    let frac = bps % 100;
    if frac == 0 {
        format!("{whole}%")
    } else if frac.is_multiple_of(10) {
        format!("{whole}.{}%", frac / 10)
    } else {
        format!("{whole}.{frac:02}%")
    }
}

/// `ceil(notional * bps / 10_000)` in micro-pUSD.
///
/// Always rounds up. This is a ceiling the owner approves and an order is
/// sized against, so truncating it would authorize less than the venue can
/// charge and leave the difference unapproved.
pub fn builder_fee_ceiling_micro(notional_micro: u64, bps: u16) -> Result<u64, String> {
    u64::try_from((u128::from(notional_micro) * u128::from(bps)).div_ceil(10_000))
        .map_err(|_| "builder fee ceiling exceeds the supported pUSD amount".to_owned())
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
    fn a_ceiling_is_bounded_by_the_venue_and_cannot_be_zero_on_both_sides() {
        // Boundaries: the venue's own maxima are accepted, one above is not.
        assert!(
            BuilderFeeCeilingBps {
                maker_bps: 50,
                taker_bps: 100
            }
            .validate()
            .is_ok()
        );
        assert!(
            BuilderFeeCeilingBps {
                maker_bps: 51,
                taker_bps: 100
            }
            .validate()
            .is_err()
        );
        assert!(
            BuilderFeeCeilingBps {
                maker_bps: 50,
                taker_bps: 101
            }
            .validate()
            .is_err()
        );
        // All-zero would declare no fee through the front door, which is the
        // state a configured code without a ceiling is refused for.
        assert!(
            BuilderFeeCeilingBps {
                maker_bps: 0,
                taker_bps: 0
            }
            .validate()
            .is_err()
        );
        // One side zero is legitimate: a builder may charge takers only.
        assert!(
            BuilderFeeCeilingBps {
                maker_bps: 0,
                taker_bps: 25
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn release_default_if_set_is_a_valid_builder_code() {
        // The default is public data declared in source, so it gets the same
        // check the operator override gets at its write route: an unparsable
        // or all-zero value would otherwise silently attach nothing, or make
        // every order fail once a release declared it.
        if let Some(code) = EMBEDDED_DEFAULT_BUILDER_CODE {
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
            EMBEDDED_DEFAULT_BUILDER_CODE.map(str::to_owned)
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
