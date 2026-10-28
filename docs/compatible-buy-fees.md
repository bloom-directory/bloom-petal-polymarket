# BUY fees on the v0.1.6 compatibility line

This patch ports the fee accounting from Polymarket #18 without its wallet
identity/signing-evidence changes. The SDK/tooling revision remains
`2beed2ff344ce2b0c112e07096027e1ae0404007`, with the same 109 routes.

BUY quotes capture Gamma's fee schedule. The local order/daily caps and warning
threshold include the estimated maximum fee. Missing, malformed, unsupported
or unexpectedly extended schedules refuse BUYs; they do not break public
market reads or SELLs. Decimal fee math uses integer units. Older BUY receipts
are counted conservatively and older drafts must be revalidated.

Revalidation and posting refresh the schedule, and posting compares it and its
bound against the reviewed values. The quote, plan, final review, prepared
approval record and receipt show the fee. Successful, rejected and ambiguous
receipts use the same fee-bearing record construction before signing.

These are Petal-side checks on the venue's published schedule. Protocol 1.7
does not gain the new Broker signing-evidence verification, and the signed
exchange order does not encode a fee ceiling. Resting orders can outlive the
schedule observed at posting. Every fee display includes that limitation;
do not advertise the estimate as an on-chain or Broker-enforced fee cap.

The previously added SELL checks remain: matching fresh preflight, sufficient
balances, operator approval and policy eligibility. The fee change neither
enables an unchecked SELL nor changes signing classes or authority route IDs.

Validation: full route tests, strict clippy, formatting, the pinned builder
and package checker. Product acceptance must still verify actual BUY debits,
SELL proceeds, refusal/retry behavior and installed-package authorization on
the combined 0.4.1 services. A build is not live trading evidence.

Source: fee implementation and compatible flow tests adapted from
`2c8658958559d694dbeaaf1ac58a432ab2139190` and its fee commit
`45fbc04dc1f2ebc9ffc77cd4b5cc6f86470e9033`.
