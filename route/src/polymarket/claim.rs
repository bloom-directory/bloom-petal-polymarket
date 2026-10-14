//! What each signed Polymarket payload moves, as the Petal use claim states it.
//!
//! Bloom turns a claim's debits into the approval's value limits, Broker checks
//! its destinations against the wallet's `allowed_destinations` by exact string
//! equality, and both are shown to the owner. Every function here derives the
//! effects from the exact payload being signed, and refuses a payload whose
//! effects it cannot state rather than signing it under a vaguer claim.
//!
//! Conventions, chosen to match what an owner's policy names:
//!
//! - The chain is Bloom's chain name for the configured chain (default
//!   `polygon`), the same name the route stages and reads on.
//! - Destinations and ERC-20 assets are EIP-55 checksummed addresses.
//! - An outcome token (an ERC-1155 position on the CTF) is
//!   `<CTF address>/<decimal token id>`.
//!
//! No claim declares a fee. Deposit-wallet batches are submitted and paid for
//! by Polymarket's relayer and CLOB orders are settled by the exchange
//! operator, so no signed payload spends the owner's POL; and Bloom catalogues
//! every Petal operation class with no fee asset, so a fee would be refused.

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::sol_types::SolCall;
use petal::{ClaimAsset, ClaimEffects};
use serde::{Deserialize, Serialize};

use crate::polymarket::eip712::{
    Batch, CTF, CTF_COLLATERAL_ADAPTER, CTF_EXCHANGE, Call, NEG_RISK_ADAPTER,
    NEG_RISK_CTF_COLLATERAL_ADAPTER, NEG_RISK_EXCHANGE, PUSD, batch_signing_hash,
};
use crate::polymarket::order::Order;
use crate::polymarket::wallet::{
    approveCall, redeemPositionsCall, setApprovalForAllCall, transferCall,
};
use crate::polymarket::{PolymarketError, Result};

/// pUSD, the CLOB collateral.
pub fn pusd_asset(chain: &str) -> ClaimAsset {
    ClaimAsset::new(chain, &PUSD.to_checksum(None))
}

/// One CTF outcome token, an ERC-1155 position id.
pub fn outcome_asset(chain: &str, token_id: U256) -> ClaimAsset {
    ClaimAsset::new(chain, &format!("{}/{token_id}", CTF.to_checksum(None)))
}

/// Spenders onboarding grants a pUSD allowance and revoke-approvals withdraws.
const PUSD_SPENDERS: [Address; 5] = [
    CTF_EXCHANGE,
    NEG_RISK_EXCHANGE,
    NEG_RISK_ADAPTER,
    CTF_COLLATERAL_ADAPTER,
    NEG_RISK_CTF_COLLATERAL_ADAPTER,
];

/// Operators onboarding approves on the CTF and revoke-approvals withdraws.
const CTF_OPERATORS: [Address; 4] = [
    CTF_EXCHANGE,
    NEG_RISK_EXCHANGE,
    CTF_COLLATERAL_ADAPTER,
    NEG_RISK_CTF_COLLATERAL_ADAPTER,
];

/// Effects of a CLOB order signed under `polymarket.order.poly1271`.
///
/// The maker is the deposit wallet. A BUY gives at most `makerAmount` pUSD and
/// a SELL at most `makerAmount` of the outcome token. Either way it settles
/// through the exchange that verifies the order, the only destination the
/// signed bytes name: the counterparty is chosen at match time.
///
/// A non-zero `builder` field attaches a fee-bearing builder code, which this
/// class cannot declare, so such an order is refused.
pub fn order_effects(chain: &str, order: &Order, neg_risk: bool) -> Result<ClaimEffects> {
    if !order.builder.is_zero() {
        return Err(PolymarketError::invalid(
            "this order carries a builder code, whose fees the trader pays; polymarket.order.poly1271 cannot declare them",
        ));
    }
    let asset = match order.side {
        0 => pusd_asset(chain),
        1 => outcome_asset(chain, order.tokenId),
        side => {
            return Err(PolymarketError::invalid(format!(
                "order side {side} is neither BUY nor SELL"
            )));
        }
    };
    if order.makerAmount.is_zero() {
        return Err(PolymarketError::invalid("order makerAmount is zero"));
    }
    let exchange = if neg_risk {
        NEG_RISK_EXCHANGE
    } else {
        CTF_EXCHANGE
    };
    Ok(ClaimEffects::moves_value()
        .debit(&asset, &order.makerAmount.to_string())
        .destination(chain, &exchange.to_checksum(None)))
}

/// Effects of a deposit-wallet batch, read call by call.
///
/// - pUSD `transfer(to, amount)` debits `amount` pUSD to `to`.
/// - pUSD `approve(spender, amount > 0)` and CTF
///   `setApprovalForAll(operator, true)` move nothing when signed, but let the
///   spender move the wallet's pUSD or positions later, so each spender is a
///   destination with no debit. An unlimited allowance has no finite debit.
/// - pUSD `approve(spender, 0)` and `setApprovalForAll(operator, false)`
///   withdraw authority: they move nothing and authorize no one.
/// - `redeemPositions` through a pUSD collateral adapter hands the condition's
///   positions to the adapter, which pays their resolved value back to the
///   deposit wallet in pUSD. The adapter is the destination. The amounts are
///   the wallet's whole position balances at execution, which the signed bytes
///   do not fix, so no debit is declared.
///
/// Anything else, including any call carrying POL, is refused.
pub fn wallet_batch_effects(chain: &str, calls: &[Call]) -> Result<ClaimEffects> {
    if calls.is_empty() {
        return Err(PolymarketError::invalid(
            "deposit-wallet batch has no calls",
        ));
    }
    let mut effects = ClaimEffects::moves_value();
    let mut declared = false;
    let mut destinations: Vec<Address> = Vec::new();
    let mut destination = |effects: ClaimEffects, address: Address| {
        if destinations.contains(&address) {
            return effects;
        }
        destinations.push(address);
        effects.destination(chain, &address.to_checksum(None))
    };
    for (index, call) in calls.iter().enumerate() {
        let invalid =
            |message: String| PolymarketError::invalid(format!("batch call {index}: {message}"));
        if !call.value.is_zero() {
            return Err(invalid("sends POL, which this Petal never signs".into()));
        }
        let selector = call.data.get(..4).unwrap_or_default();
        if call.target == PUSD && selector == transferCall::SELECTOR {
            let transfer =
                transferCall::abi_decode(&call.data).map_err(|err| invalid(err.to_string()))?;
            if transfer.amount.is_zero() {
                return Err(invalid("transfers zero pUSD".into()));
            }
            effects = destination(
                effects.debit(&pusd_asset(chain), &transfer.amount.to_string()),
                transfer.to,
            );
            declared = true;
        } else if call.target == PUSD && selector == approveCall::SELECTOR {
            let approve =
                approveCall::abi_decode(&call.data).map_err(|err| invalid(err.to_string()))?;
            if !PUSD_SPENDERS.contains(&approve.spender) {
                return Err(invalid(format!(
                    "approves pUSD to unknown spender {}",
                    approve.spender.to_checksum(None)
                )));
            }
            if !approve.amount.is_zero() {
                effects = destination(effects, approve.spender);
                declared = true;
            }
        } else if call.target == CTF && selector == setApprovalForAllCall::SELECTOR {
            let approval = setApprovalForAllCall::abi_decode(&call.data)
                .map_err(|err| invalid(err.to_string()))?;
            if !CTF_OPERATORS.contains(&approval.operator) {
                return Err(invalid(format!(
                    "approves CTF to unknown operator {}",
                    approval.operator.to_checksum(None)
                )));
            }
            if approval.approved {
                effects = destination(effects, approval.operator);
                declared = true;
            }
        } else if matches!(
            call.target,
            CTF_COLLATERAL_ADAPTER | NEG_RISK_CTF_COLLATERAL_ADAPTER
        ) && selector == redeemPositionsCall::SELECTOR
        {
            let redeem = redeemPositionsCall::abi_decode(&call.data)
                .map_err(|err| invalid(err.to_string()))?;
            if redeem.collateralToken != PUSD {
                return Err(invalid("redeems into a collateral other than pUSD".into()));
            }
            effects = destination(effects, call.target);
            declared = true;
        } else {
            return Err(invalid(format!(
                "a call to {} has effects this Petal cannot state",
                call.target.to_checksum(None)
            )));
        }
    }
    Ok(if declared {
        effects
    } else {
        ClaimEffects::moves_nothing()
    })
}

/// One call of a deposit-wallet batch as the route persists it and sends it to
/// the relayer: an EIP-55 `target`, a decimal `value` and 0x-hex `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchCall {
    #[serde(with = "alloy::serde::checksum")]
    pub target: Address,
    #[serde(with = "alloy::serde::displayfromstr")]
    pub value: U256,
    pub data: Bytes,
}

impl From<&Call> for BatchCall {
    fn from(call: &Call) -> Self {
        Self {
            target: call.target,
            value: call.value,
            data: call.data.clone(),
        }
    }
}

impl From<&BatchCall> for Call {
    fn from(call: &BatchCall) -> Self {
        Self {
            target: call.target,
            value: call.value,
            data: call.data.clone(),
        }
    }
}

/// A prepared deposit-wallet batch as `PreparedSigning.preimage` persists it.
///
/// It serializes to the same JSON the route has always written, so stored
/// records and the review hashes taken over them stay valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedBatch {
    #[serde(with = "alloy::serde::checksum")]
    pub deposit_wallet: Address,
    pub chain_id: u64,
    pub nonce: u64,
    pub deadline: u64,
    pub calls: Vec<BatchCall>,
    pub review_intent_hash: String,
}

impl PreparedBatch {
    pub fn from_preimage(preimage: &serde_json::Value) -> Result<Self> {
        Self::deserialize(preimage)
            .map_err(|err| PolymarketError::signing(format!("prepared batch: {err}")))
    }

    pub fn to_preimage(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self)
            .map_err(|err| PolymarketError::signing(format!("encode prepared batch: {err}")))
    }
}

/// Effects of a prepared deposit-wallet batch.
///
/// The batch is rebuilt and its EIP-712 hash compared with `signing_hash`, so
/// the effects describe the exact bytes being signed rather than whatever the
/// persisted record says. A record that does not match is corrupt.
pub fn prepared_batch_effects(
    chain: &str,
    prepared: &PreparedBatch,
    signing_hash: B256,
) -> Result<ClaimEffects> {
    let batch = Batch {
        wallet: prepared.deposit_wallet,
        nonce: U256::from(prepared.nonce),
        deadline: U256::from(prepared.deadline),
        calls: prepared.calls.iter().map(Call::from).collect(),
    };
    if batch_signing_hash(&batch, prepared.chain_id, prepared.deposit_wallet) != signing_hash {
        return Err(PolymarketError::signing(
            "prepared batch calls do not match the hash being signed",
        ));
    }
    wallet_batch_effects(chain, &batch.calls)
}

#[cfg(test)]
mod tests {
    use petal::{ClaimDebit, ClaimDestination, ClaimFee};

    use super::*;
    use crate::polymarket::order::{LimitQuote, OrderParams, SIG_TYPE_POLY_1271, build_order};
    use crate::polymarket::types::Side;
    use crate::polymarket::wallet::{
        approval_calls, approve_call, redeem_positions_call, revoke_calls,
        set_approval_for_all_call, transfer_amount_call,
    };

    const CHAIN: &str = "polygon";
    const DEPOSIT: Address = Address::repeat_byte(0x42);

    fn order(side: Side, builder: Option<B256>) -> Order {
        let quote = match side {
            Side::Buy => LimitQuote::buy_from_spend(10_000_000, 500_000, 10_000).unwrap(),
            Side::Sell => LimitQuote::sell_from_shares(20_000_000, 500_000, 10_000).unwrap(),
        };
        build_order(
            &OrderParams {
                token_id: U256::from(7_777u64),
                maker: DEPOSIT,
                quote,
                builder_code: builder,
                signature_type: SIG_TYPE_POLY_1271,
            },
            1_700_000_000_000,
        )
    }

    fn debit(asset: &str, amount: &str) -> ClaimDebit {
        ClaimDebit {
            asset: ClaimAsset::new(CHAIN, asset),
            amount: amount.into(),
        }
    }

    fn destination(address: &str) -> ClaimDestination {
        ClaimDestination {
            chain: CHAIN.into(),
            destination: address.into(),
        }
    }

    /// The exact strings an owner's `allowed_destinations` must carry. Written
    /// out rather than derived, so a change to an address or its casing fails.
    const CTF_EXCHANGE_STR: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
    const NEG_RISK_EXCHANGE_STR: &str = "0xe2222d279d744050d28e00520010520000310F59";
    const NEG_RISK_ADAPTER_STR: &str = "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296";
    const CTF_COLLATERAL_ADAPTER_STR: &str = "0xAdA100Db00Ca00073811820692005400218FcE1f";
    const NEG_RISK_CTF_COLLATERAL_ADAPTER_STR: &str = "0xadA2005600Dec949baf300f4C6120000bDB6eAab";
    const PUSD_STR: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";

    #[test]
    fn declared_addresses_are_eip55_checksummed() {
        for (address, expected) in [
            (CTF_EXCHANGE, CTF_EXCHANGE_STR),
            (NEG_RISK_EXCHANGE, NEG_RISK_EXCHANGE_STR),
            (NEG_RISK_ADAPTER, NEG_RISK_ADAPTER_STR),
            (CTF_COLLATERAL_ADAPTER, CTF_COLLATERAL_ADAPTER_STR),
            (
                NEG_RISK_CTF_COLLATERAL_ADAPTER,
                NEG_RISK_CTF_COLLATERAL_ADAPTER_STR,
            ),
            (PUSD, PUSD_STR),
        ] {
            assert_eq!(address.to_checksum(None), expected);
            assert_eq!(Address::parse_checksummed(expected, None).unwrap(), address);
        }
    }

    #[test]
    fn a_buy_debits_its_pusd_maker_amount_to_the_exchange() {
        let buy = order(Side::Buy, None);
        assert_eq!(buy.makerAmount, U256::from(10_000_000u64));
        let effects = order_effects(CHAIN, &buy, false).unwrap();
        effects.validate().unwrap();
        assert_eq!(effects.debits(), &[debit(PUSD_STR, "10000000")]);
        assert_eq!(effects.destinations(), &[destination(CTF_EXCHANGE_STR)]);
        assert_eq!(effects.declared_fee(), &ClaimFee::None);
        let neg_risk = order_effects(CHAIN, &buy, true).unwrap();
        assert_eq!(
            neg_risk.destinations(),
            &[destination(NEG_RISK_EXCHANGE_STR)]
        );
    }

    #[test]
    fn a_sell_debits_its_outcome_tokens_to_the_exchange() {
        let sell = order(Side::Sell, None);
        let effects = order_effects(CHAIN, &sell, false).unwrap();
        effects.validate().unwrap();
        assert_eq!(
            effects.debits(),
            &[debit(
                "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045/7777",
                "20000000"
            )]
        );
        assert_eq!(effects.destinations(), &[destination(CTF_EXCHANGE_STR)]);
    }

    #[test]
    fn a_builder_coded_order_is_refused() {
        let coded = order(Side::Buy, Some(B256::repeat_byte(0xbc)));
        let err = order_effects(CHAIN, &coded, false).unwrap_err().to_string();
        assert!(err.contains("builder code"), "{err}");
    }

    #[test]
    fn onboarding_approvals_name_every_spender_once() {
        let effects = wallet_batch_effects(CHAIN, &approval_calls()).unwrap();
        effects.validate().unwrap();
        assert!(effects.debits().is_empty());
        assert_eq!(
            effects.destinations(),
            &[
                destination(CTF_EXCHANGE_STR),
                destination(NEG_RISK_EXCHANGE_STR),
                destination(NEG_RISK_ADAPTER_STR),
                destination(CTF_COLLATERAL_ADAPTER_STR),
                destination(NEG_RISK_CTF_COLLATERAL_ADAPTER_STR),
            ]
        );
        assert_eq!(effects.declared_fee(), &ClaimFee::None);
    }

    #[test]
    fn a_withdrawal_debits_pusd_to_its_recipient() {
        let owner = Address::repeat_byte(0x11);
        let effects = wallet_batch_effects(
            CHAIN,
            &[transfer_amount_call(PUSD, owner, U256::from(1_250_000u64))],
        )
        .unwrap();
        effects.validate().unwrap();
        assert_eq!(effects.debits(), &[debit(PUSD_STR, "1250000")]);
        assert_eq!(
            effects.destinations(),
            &[destination(&owner.to_checksum(None))]
        );
        assert!(
            wallet_batch_effects(CHAIN, &[transfer_amount_call(PUSD, owner, U256::ZERO)]).is_err()
        );
    }

    #[test]
    fn revocations_move_nothing() {
        assert_eq!(
            wallet_batch_effects(CHAIN, &revoke_calls()).unwrap(),
            ClaimEffects::moves_nothing()
        );
    }

    #[test]
    fn a_redemption_names_its_collateral_adapter() {
        for (neg_risk, adapter) in [
            (false, CTF_COLLATERAL_ADAPTER_STR),
            (true, NEG_RISK_CTF_COLLATERAL_ADAPTER_STR),
        ] {
            let effects = wallet_batch_effects(
                CHAIN,
                &[redeem_positions_call(B256::repeat_byte(1), neg_risk)],
            )
            .unwrap();
            effects.validate().unwrap();
            assert!(effects.debits().is_empty());
            assert_eq!(effects.destinations(), &[destination(adapter)]);
        }
    }

    #[test]
    fn a_batch_call_whose_effects_are_unknown_is_refused() {
        let stranger = Address::repeat_byte(0x99);
        let refused = [
            vec![],
            vec![approve_call(PUSD, stranger)],
            vec![set_approval_for_all_call(CTF, stranger)],
            vec![transfer_amount_call(
                crate::polymarket::eip712::USDC_E,
                stranger,
                U256::from(1u8),
            )],
            vec![Call {
                target: PUSD,
                value: U256::from(1u8),
                data: Default::default(),
            }],
        ];
        for (case, calls) in refused.iter().enumerate() {
            assert!(wallet_batch_effects(CHAIN, calls).is_err(), "case {case}");
        }
    }

    #[test]
    fn a_prepared_batch_is_checked_against_the_hash_being_signed() {
        let calls = [transfer_amount_call(
            PUSD,
            Address::repeat_byte(0x11),
            U256::from(5u8),
        )];
        let built = crate::polymarket::signing::wallet_batch_action_and_hash(
            DEPOSIT,
            137,
            3,
            1_900_000_000,
            &calls,
        );
        let prepared = PreparedBatch {
            deposit_wallet: DEPOSIT,
            chain_id: 137,
            nonce: 3,
            deadline: 1_900_000_000,
            calls: calls.iter().map(BatchCall::from).collect(),
            review_intent_hash: "review".into(),
        };
        let effects = prepared_batch_effects(CHAIN, &prepared, built.signing_hash).unwrap();
        assert_eq!(effects.debits(), &[debit(PUSD_STR, "5")]);
        let mut tampered = prepared.clone();
        tampered.calls.clear();
        assert!(prepared_batch_effects(CHAIN, &tampered, built.signing_hash).is_err());
        assert!(prepared_batch_effects(CHAIN, &prepared, B256::ZERO).is_err());
    }

    struct Post;
    impl petal::RouteIdentity for Post {
        const PATH: &'static str = "trade/[wallet]/[index]/drafts/[id]/post";
        const CANONICAL_PATH: &'static str = "trade/[wallet]/[index]/drafts/[id]/post";
        const PARAMS: &'static [(&'static str, usize)] = &[("wallet", 1), ("index", 2), ("id", 4)];
    }

    /// What reaches Bloom: the SDK claim carries these effects unchanged, and
    /// the batch request carries the claim's class, suite and payload.
    #[test]
    fn the_signed_claim_carries_the_order_effects() {
        let ctx = petal::testing::RouteCtx::new("trade/main/0/drafts/d1/post")
            .account("main", 0)
            .bind::<Post>();
        let buy = order(Side::Buy, None);
        let effects = order_effects(CHAIN, &buy, false).unwrap();
        let preimage = crate::polymarket::order::poly1271_signing_preimage(&buy, 137, false);
        let payload = petal::PayloadSignItem {
            claimed_hash: alloy::primitives::keccak256(&preimage).into(),
            preimage,
        };
        let claim = petal::petal_use_claim(
            &ctx,
            "polymarket.order.poly1271",
            petal::CryptoSuite::Secp256k1Keccak256Recoverable,
            std::slice::from_ref(&payload),
            &effects,
            petal::ClaimNonce::Derived,
        )
        .unwrap();
        assert_eq!(ClaimEffects::from_claim_jcs(claim.jcs()).unwrap(), effects);
        let request = claim.batch_request("main", petal::SignSelector::Exact);
        assert_eq!(request.operation_class, "polymarket.order.poly1271");
        assert_eq!(
            request.signature_algorithm,
            "secp256k1-keccak256-recoverable"
        );
        assert_eq!(request.payloads, vec![payload]);
        assert_eq!(request.petal_use_claim_jcs, claim.jcs());
    }

    /// The record must serialize to exactly the JSON the route wrote before it
    /// was typed: stored preimages and the review hashes over them depend on it.
    #[test]
    fn a_prepared_batch_keeps_its_persisted_json() {
        let calls = approval_calls();
        let legacy = serde_json::json!({
            "deposit_wallet": DEPOSIT.to_checksum(None),
            "chain_id": 137,
            "nonce": 3,
            "deadline": 1_900_000_000u64,
            "calls": calls
                .iter()
                .map(|call| {
                    serde_json::json!({
                        "target": call.target.to_checksum(None),
                        "value": call.value.to_string(),
                        "data": format!("0x{}", hex::encode(call.data.as_ref())),
                    })
                })
                .collect::<Vec<_>>(),
            "review_intent_hash": "review",
        });
        let prepared = PreparedBatch::from_preimage(&legacy).unwrap();
        assert_eq!(prepared.calls.len(), calls.len());
        for (read, call) in prepared.calls.iter().zip(&calls) {
            assert_eq!(
                (read.target, read.value, &read.data),
                (call.target, call.value, &call.data)
            );
        }
        let rewritten = prepared.to_preimage().unwrap();
        assert_eq!(rewritten, legacy);
        assert_eq!(
            serde_json::to_vec(&rewritten).unwrap(),
            serde_json::to_vec(&legacy).unwrap()
        );

        // Targets are written checksummed, so a lowercase one is corrupt.
        let mut lowercase = legacy.clone();
        lowercase["calls"][0]["target"] = format!("{:#x}", calls[0].target).into();
        assert!(PreparedBatch::from_preimage(&lowercase).is_err());
        assert!(PreparedBatch::from_preimage(&serde_json::json!({})).is_err());
    }
}
