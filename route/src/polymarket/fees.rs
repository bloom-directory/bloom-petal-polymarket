//! Polymarket's trading fee, and the most a signed BUY can be charged.
//!
//! Polymarket publishes its fee as `fee = C × rate × p × (1 − p)` in pUSD,
//! where `C` is the shares traded and `p` the trade price
//! (<https://docs.polymarket.com/trading/fees.md>). Gamma publishes a market's
//! schedule as `feesEnabled`, `feeType` and `feeSchedule`, for example
//! `{"exponent":1,"rate":0.07,"takerOnly":true,"rebateRate":0.2}`. The
//! `makerBaseFee` / `takerBaseFee` fields are not the rate charged.
//!
//! A signed CLOB order carries no fee. The exchange operator chooses the fee
//! at match time, and the exchange pulls a BUY's fee from the maker in pUSD
//! together with its principal. The exchange's on-chain `maxFeeRate` is 0,
//! which disables the on-chain cap. The bound computed here therefore holds
//! only while the operator charges the published schedule.
//!
//! Whatever price a charge is taken at, `C × p` is the pUSD `F` the order
//! spends in it, so the charge is `F × rate × (1 − p)`, less than
//! `F × rate`. An order's matches spend at most its `makerAmount` in total,
//! so its fees sum to less than `makerAmount × rate` however low it fills.
//! The bound does not depend on the market's tick: Polymarket can shrink a
//! tick (0.01 to 0.001, down to 0.0001) between the quote and the match, and
//! a BUY that fills below the quoted tick pays a larger fee.

use serde::{Deserialize, Serialize};

use crate::polymarket::order::{MICRO, format_micro, parse_micro};
use crate::polymarket::types::Market;
use crate::polymarket::{PolymarketError, Result};

/// The highest `feeSchedule.rate` accepted, in parts per million (0.25).
///
/// Live schedules use rates from 0.03 to 0.07, and a BUY's bound is `rate`
/// times its pUSD. Above 0.25 the bound would exceed a quarter of the order.
/// A rate that high is more likely a unit change in Gamma's feed than a real
/// fee, so BUYs stop until someone checks it.
pub const MAX_FEE_RATE_PPM: u64 = 250_000;

/// pUSD units added to a non-zero fee bound. Defensive: no charge observed
/// needs it.
///
/// Charges sampled on chain are rounded down to 0.00001 pUSD (10 units), and
/// `makerAmount × rate` already exceeds the exact fee at any fill price. The
/// margin covers what the published formula leaves open: a charge rounded up
/// to Polymarket's 5-decimal fee precision (under 10 units), or priced from a
/// rounded price so that `C × p` exceeds `F` by integer truncation (under 1
/// unit per fill). 1,000 units cover about 100 such charges against one
/// order, and add 0.001 pUSD to the value limit.
pub const FEE_ROUNDING_MARGIN_MICRO: u64 = 1_000;

/// One line for every surface that shows the fee bound to the owner.
pub const FEE_BOUND_NOTE: &str = "The fee bound is enforced by Polymarket's operator, not on chain: the signed order carries no fee, and the exchange takes a BUY's fee from the deposit wallet in pUSD (a SELL's from its proceeds) at match time.";

/// The fee a market charges, as Gamma's `feesEnabled` and `feeSchedule`
/// state it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketFee {
    /// Gamma reports `feesEnabled: false`.
    Disabled,
    /// Gamma reports `feesEnabled: true` with an exponent-1 schedule.
    Enabled {
        /// Gamma's `feeType`. Shown to the owner, never interpreted: the
        /// formula is the same for every type.
        fee_type: Option<String>,
        /// `feeSchedule.rate` in parts per million (`0.07` is `70000`).
        rate_ppm: u64,
        /// `feeSchedule.takerOnly`. The bound holds for makers too, so this
        /// is shown, not relied on.
        taker_only: bool,
    },
    /// Gamma's fee data does not let this Petal bound a BUY's fee. A BUY is
    /// refused; a SELL is not, because its fee comes out of its proceeds.
    Unbounded {
        /// Gamma's `feeType`, shown to the owner.
        fee_type: Option<String>,
        /// Why the fee cannot be bounded.
        reason: String,
    },
}

/// Gamma's `feeSchedule`. serde_json is built without `arbitrary_precision`,
/// so each number is parsed as an `f64` and printed back as its shortest
/// round-trip decimal (`0.070` reads as `0.07`). That decimal is then parsed
/// exactly into parts per million, so any rate with at most 6 decimal places
/// is read exactly, and anything else is refused rather than rounded.
#[derive(Debug, Deserialize)]
struct GammaFeeSchedule {
    exponent: serde_json::Number,
    rate: serde_json::Number,
    #[serde(rename = "takerOnly", default)]
    taker_only: bool,
}

impl MarketFee {
    /// Read a market's fee from Gamma. A market whose fee this module cannot
    /// bound reads as [`MarketFee::Unbounded`]: no `feesEnabled`, a
    /// `feesEnabled` that is not a JSON boolean, or fees enabled with a
    /// missing or unreadable schedule, an exponent other than 1, or a rate
    /// that is not an exact decimal of at most 6 places in `[0, 0.25]`. A
    /// `feeType` that is not a string is shown as no fee type.
    pub fn from_market(market: &Market) -> Self {
        Self::read(market).unwrap_or_else(|reason| Self::Unbounded {
            fee_type: gamma_fee_type(market),
            reason,
        })
    }

    fn read(market: &Market) -> std::result::Result<Self, String> {
        match &market.fees_enabled {
            None => Err(
                "Gamma does not say whether the market charges a trading fee (no feesEnabled)"
                    .into(),
            ),
            Some(serde_json::Value::Bool(false)) => Ok(Self::Disabled),
            Some(serde_json::Value::Bool(true)) => {
                let raw = market
                    .fee_schedule
                    .as_ref()
                    .ok_or("Gamma reports feesEnabled but no feeSchedule")?;
                let schedule = GammaFeeSchedule::deserialize(raw)
                    .map_err(|err| format!("Gamma's feeSchedule is unreadable: {err}"))?;
                let exponent = schedule.exponent.to_string();
                if parse_micro(&exponent).ok() != Some(MICRO) {
                    return Err(format!(
                        "feeSchedule exponent {exponent} is not 1, the only fee curve this Petal bounds"
                    ));
                }
                let rate = schedule.rate.to_string();
                let rate_ppm = parse_micro(&rate).map_err(|_| {
                    format!(
                        "feeSchedule rate {rate} is not a non-negative decimal with at most 6 places"
                    )
                })?;
                if rate_ppm > MAX_FEE_RATE_PPM {
                    return Err(format!(
                        "feeSchedule rate {rate} is above {}, which this Petal treats as implausible",
                        format_micro(MAX_FEE_RATE_PPM)
                    ));
                }
                Ok(Self::Enabled {
                    fee_type: gamma_fee_type(market),
                    rate_ppm,
                    taker_only: schedule.taker_only,
                })
            }
            Some(other) => Err(format!("Gamma's feesEnabled is {other}, not a boolean")),
        }
    }

    /// The most pUSD, in micro-units, this market's schedule lets the operator
    /// charge a BUY whose signed `makerAmount` is `maker_micro`:
    /// `ceil(maker_micro × rate)` plus [`FEE_ROUNDING_MARGIN_MICRO`], exact in
    /// `u128`. That is the fee as the fill price approaches zero, so price
    /// improvement and tick changes cannot exceed it (see the module docs).
    /// Refused for a market whose fee cannot be bounded.
    pub fn buy_fee_cap(&self, maker_micro: u64) -> Result<u64> {
        match self {
            Self::Disabled => Ok(0),
            Self::Enabled { rate_ppm, .. } => fee_cap_at_rate(maker_micro, *rate_ppm),
            Self::Unbounded { reason, .. } => Err(PolymarketError::invalid(format!(
                "{reason}; refusing to sign a BUY whose fee cannot be bounded"
            ))),
        }
    }
}

/// Gamma's `feeType` label, or `None` when it is missing or not a string.
fn gamma_fee_type(market: &Market) -> Option<String> {
    market
        .fee_type
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The fee bound for a BUY of `maker_micro` whose market's schedule was not
/// recorded (a receipt written before fees were bounded): the bound at
/// [`MAX_FEE_RATE_PPM`], the highest rate this Petal accepts.
pub fn max_buy_fee_cap(maker_micro: u64) -> u64 {
    fee_cap_at_rate(maker_micro, MAX_FEE_RATE_PPM).unwrap_or(u64::MAX)
}

fn fee_cap_at_rate(maker_micro: u64, rate_ppm: u64) -> Result<u64> {
    if rate_ppm == 0 {
        return Ok(0);
    }
    let exact_cap = (u128::from(maker_micro) * u128::from(rate_ppm)).div_ceil(u128::from(MICRO));
    u64::try_from(exact_cap)
        .ok()
        .and_then(|cap| cap.checked_add(FEE_ROUNDING_MARGIN_MICRO))
        .ok_or_else(|| PolymarketError::invalid("order fee bound overflows"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `will-bitcoin-dip-to-77pt5k-in-october-2026` as Gamma's
    /// `/markets/slug/<slug>` returned it, trimmed to the fields that matter.
    pub(crate) const FEE_ENABLED_MARKET: &str = r#"{
        "slug": "will-bitcoin-dip-to-77pt5k-in-october-2026",
        "question": "Will Bitcoin dip to $77,500 in October 2026?",
        "conditionId": "0xabc",
        "clobTokenIds": "[\"1\", \"2\"]",
        "outcomes": "[\"Yes\", \"No\"]",
        "active": true,
        "closed": false,
        "enableOrderBook": true,
        "orderPriceMinTickSize": 0.01,
        "orderMinSize": 5,
        "negRisk": false,
        "feesEnabled": true,
        "feeType": "crypto_fees_v2",
        "feeSchedule": {"exponent": 1, "rate": 0.07, "takerOnly": true, "rebateRate": 0.2},
        "makerBaseFee": 1000,
        "takerBaseFee": 1000
    }"#;

    /// `will-the-us-invade-iran-before-2027`: no fee, no `feeSchedule` field.
    const FEE_FREE_MARKET: &str = r#"{
        "slug": "will-the-us-invade-iran-before-2027",
        "conditionId": "0xdef",
        "clobTokenIds": "[\"3\", \"4\"]",
        "outcomes": "[\"Yes\", \"No\"]",
        "orderPriceMinTickSize": 0.01,
        "negRisk": false,
        "feesEnabled": false,
        "feeType": null
    }"#;

    fn market(raw: &str) -> Market {
        serde_json::from_str(raw).unwrap()
    }

    fn with(edit: impl FnOnce(&mut serde_json::Value)) -> MarketFee {
        let mut value: serde_json::Value = serde_json::from_str(FEE_ENABLED_MARKET).unwrap();
        edit(&mut value);
        MarketFee::from_market(&serde_json::from_value(value).unwrap())
    }

    const CRYPTO: MarketFee = MarketFee::Enabled {
        fee_type: None,
        rate_ppm: 70_000,
        taker_only: true,
    };

    #[test]
    fn a_fee_enabled_gamma_market_reads_its_schedule() {
        let market = market(FEE_ENABLED_MARKET);
        assert_eq!(market.fees_enabled, Some(true.into()));
        assert_eq!(market.fee_type, Some("crypto_fees_v2".into()));
        assert_eq!(
            MarketFee::from_market(&market),
            MarketFee::Enabled {
                fee_type: Some("crypto_fees_v2".into()),
                rate_ppm: 70_000,
                taker_only: true,
            }
        );
    }

    #[test]
    fn a_fee_free_gamma_market_reads_as_disabled() {
        let market = market(FEE_FREE_MARKET);
        assert_eq!(market.fees_enabled, Some(false.into()));
        assert_eq!(market.fee_schedule, None);
        assert_eq!(MarketFee::from_market(&market), MarketFee::Disabled);

        let mut value: serde_json::Value = serde_json::from_str(FEE_FREE_MARKET).unwrap();
        value["feeSchedule"] = serde_json::Value::Null;
        let market: Market = serde_json::from_value(value).unwrap();
        assert_eq!(MarketFee::from_market(&market), MarketFee::Disabled);
    }

    #[test]
    fn rates_are_read_exactly_and_maker_fees_are_accepted() {
        assert_eq!(
            with(|m| {
                m["feeSchedule"]["rate"] = serde_json::json!(0.0725);
                m["feeSchedule"]["takerOnly"] = false.into();
            }),
            MarketFee::Enabled {
                fee_type: Some("crypto_fees_v2".into()),
                rate_ppm: 72_500,
                taker_only: false,
            }
        );
        let zero = with(|m| m["feeSchedule"]["rate"] = 0.into());
        assert!(matches!(zero, MarketFee::Enabled { rate_ppm: 0, .. }));
        let at_max = with(|m| m["feeSchedule"]["rate"] = serde_json::json!(0.25));
        assert!(matches!(
            at_max,
            MarketFee::Enabled {
                rate_ppm: 250_000,
                ..
            }
        ));
        // An unknown feeType is shown, not refused.
        let named = with(|m| m["feeType"] = "brand_new_fees".into());
        assert!(
            matches!(named, MarketFee::Enabled { fee_type: Some(t), .. } if t == "brand_new_fees")
        );
    }

    #[test]
    fn schedules_that_cannot_be_bounded_refuse_a_buy() {
        let unbounded = [
            (
                "no feesEnabled",
                with(|m| {
                    m.as_object_mut().unwrap().remove("feesEnabled");
                }),
            ),
            (
                "no feeSchedule",
                with(|m| {
                    m.as_object_mut().unwrap().remove("feeSchedule");
                }),
            ),
            (
                "null feeSchedule",
                with(|m| m["feeSchedule"] = serde_json::Value::Null),
            ),
            (
                "schedule is a string",
                with(|m| m["feeSchedule"] = "0.07".into()),
            ),
            (
                "no rate",
                with(|m| {
                    m["feeSchedule"].as_object_mut().unwrap().remove("rate");
                }),
            ),
            (
                "rate is a string",
                with(|m| m["feeSchedule"]["rate"] = "0.07".into()),
            ),
            (
                "exponent 2",
                with(|m| m["feeSchedule"]["exponent"] = 2.into()),
            ),
            (
                "exponent 0.5",
                with(|m| m["feeSchedule"]["exponent"] = serde_json::json!(0.5)),
            ),
            (
                "negative rate",
                with(|m| m["feeSchedule"]["rate"] = serde_json::json!(-0.07)),
            ),
            (
                "rate above 0.25",
                with(|m| m["feeSchedule"]["rate"] = serde_json::json!(0.3)),
            ),
            ("rate of 1", with(|m| m["feeSchedule"]["rate"] = 1.into())),
            (
                "seven decimals",
                with(|m| m["feeSchedule"]["rate"] = serde_json::json!(0.0700001)),
            ),
        ];
        for (case, fee) in unbounded {
            let MarketFee::Unbounded { fee_type, .. } = &fee else {
                panic!("{case}: read as {fee:?}");
            };
            assert_eq!(fee_type.as_deref(), Some("crypto_fees_v2"), "{case}");
            let err = fee.buy_fee_cap(2_000_000).unwrap_err().to_string();
            assert!(
                err.contains("refusing to sign a BUY whose fee cannot be bounded"),
                "{case}: {err}"
            );
        }
        let err = with(|m| m["feeSchedule"]["exponent"] = 2.into())
            .buy_fee_cap(2_000_000)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exponent 2 is not 1"), "{err}");
        let err = with(|m| {
            m.as_object_mut().unwrap().remove("feesEnabled");
        })
        .buy_fee_cap(2_000_000)
        .unwrap_err()
        .to_string();
        assert!(err.contains("no feesEnabled"), "{err}");
    }

    /// Gamma retyping `feesEnabled` or `feeType` must not break the market
    /// read itself: quoting, the public market reads and redeem all
    /// deserialize [`Market`]. Only a BUY, whose fee must be bounded, stops.
    #[test]
    fn a_retyped_fee_field_refuses_a_buy_without_breaking_the_market_read() {
        for (field, value) in [
            ("feesEnabled", serde_json::json!("true")),
            ("feesEnabled", serde_json::json!(1)),
            ("feeType", serde_json::json!({"name": "crypto"})),
        ] {
            let mut raw: serde_json::Value = serde_json::from_str(FEE_ENABLED_MARKET).unwrap();
            raw[field] = value.clone();
            let market: Market = serde_json::from_value(raw)
                .unwrap_or_else(|err| panic!("{field}={value} breaks the market read: {err}"));
            assert_eq!(market.condition_id, "0xabc", "{field}={value}");
            assert!(market.is_binary(), "{field}={value}");
        }

        for value in [
            serde_json::json!("true"),
            serde_json::json!("false"),
            serde_json::json!(1),
            serde_json::json!(0),
            serde_json::json!({}),
        ] {
            let fee = with(|m| m["feesEnabled"] = value.clone());
            let MarketFee::Unbounded { fee_type, reason } = &fee else {
                panic!("feesEnabled={value}: read as {fee:?}");
            };
            assert_eq!(fee_type.as_deref(), Some("crypto_fees_v2"), "{value}");
            assert!(reason.contains("feesEnabled"), "{value}: {reason}");
            let err = fee.buy_fee_cap(2_000_000).unwrap_err().to_string();
            assert!(err.contains("feesEnabled"), "{value}: {err}");
            assert!(
                err.contains("refusing to sign a BUY whose fee cannot be bounded"),
                "{value}: {err}"
            );
        }

        // A feeType that is not a string is shown as none; the fee is read.
        assert_eq!(
            with(|m| m["feeType"] = serde_json::json!({"name": "crypto"})),
            MarketFee::Enabled {
                fee_type: None,
                rate_ppm: 70_000,
                taker_only: true,
            }
        );
        let unbounded = with(|m| {
            m["feesEnabled"] = "true".into();
            m["feeType"] = 7.into();
        });
        assert!(matches!(
            unbounded,
            MarketFee::Unbounded { fee_type: None, .. }
        ));
    }

    /// The live fill: BUY 5 @ 0.40 spent 2,000,000 and was charged 84,000.
    #[test]
    fn the_bound_covers_the_live_fill() {
        let cap = CRYPTO.buy_fee_cap(2_000_000).unwrap();
        // ceil(2,000,000 × 0.07) = 140,000, plus the rounding margin.
        assert_eq!(cap, 140_000 + FEE_ROUNDING_MARGIN_MICRO);
        assert_eq!(cap, 141_000);
        assert!(84_000 <= cap);
    }

    #[test]
    fn the_bound_rounds_up() {
        // 1,000,001 × 0.07 = 70,000.07.
        assert_eq!(CRYPTO.buy_fee_cap(1_000_001).unwrap(), 70_001 + 1_000);
        // 1 × 0.07 = 0.07.
        assert_eq!(CRYPTO.buy_fee_cap(1).unwrap(), 1 + 1_000);
    }

    #[test]
    fn no_fee_means_no_bound() {
        assert_eq!(MarketFee::Disabled.buy_fee_cap(2_000_000).unwrap(), 0);
        let zero = MarketFee::Enabled {
            fee_type: None,
            rate_ppm: 0,
            taker_only: true,
        };
        assert_eq!(zero.buy_fee_cap(2_000_000).unwrap(), 0);
        let max = MarketFee::Enabled {
            fee_type: None,
            rate_ppm: MAX_FEE_RATE_PPM,
            taker_only: true,
        };
        assert_eq!(
            max.buy_fee_cap(u64::MAX).unwrap(),
            u64::MAX / 4 + 1 + FEE_ROUNDING_MARGIN_MICRO
        );
        let absurd = MarketFee::Enabled {
            fee_type: None,
            rate_ppm: 2 * MICRO,
            taker_only: true,
        };
        assert!(absurd.buy_fee_cap(u64::MAX).is_err());
    }

    #[test]
    fn an_unrecorded_schedule_is_bounded_at_the_highest_accepted_rate() {
        // 2,000,000 × 0.25 = 500,000.
        assert_eq!(max_buy_fee_cap(2_000_000), 501_000);
        assert_eq!(max_buy_fee_cap(0), FEE_ROUNDING_MARGIN_MICRO);
        assert_eq!(
            max_buy_fee_cap(u64::MAX),
            u64::MAX / 4 + 1 + FEE_ROUNDING_MARGIN_MICRO
        );
    }

    /// Price improvement raises a BUY's fee, and so does a tick that shrinks
    /// after the quote: a bound at the limit price (84,000) is beaten by a
    /// fill at 0.30, and a bound at the quoted tick 0.01 (139,600) by a fill
    /// at 0.001 after the tick shrinks. The bound covers every fill price on
    /// Polymarket's finest tick, rounded up to 0.00001 pUSD.
    #[test]
    fn no_fill_price_exceeds_the_bound() {
        let maker = 2_000_000u128;
        let rate = 70_000u128;
        let cap = u128::from(CRYPTO.buy_fee_cap(2_000_000).unwrap());
        let fee_at = |price: u128| {
            let exact = (maker * rate * (1_000_000 - price)).div_ceil(1_000_000_000_000);
            exact.div_ceil(10) * 10
        };
        for price in (100..1_000_000).step_by(100) {
            assert!(fee_at(price) <= cap, "fill at {price}");
        }
        assert_eq!(fee_at(400_000), 84_000);
        assert_eq!(fee_at(300_000), 98_000);
        assert_eq!(fee_at(1_000), 139_860);
        assert!(fee_at(1_000) > u128::from(138_600 + FEE_ROUNDING_MARGIN_MICRO));
        assert_eq!(fee_at(100), 139_990);
    }
}
