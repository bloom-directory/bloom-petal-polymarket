# Polymarket Petal

Onboarding has two authority routes: write `onboard/<wallet>/<index>/begin`
to obtain CLOB credentials, then write `onboard/<wallet>/<index>/continue`
to deploy the deposit wallet. `continue` does not fund it: while the wallet
holds no pUSD it stops at stage `fund`. Fund it with
`fund/<wallet>/<index>/new`, then write `continue` again to approve.
`status.json` names the next action at every stage. With existing
credentials, `begin` acknowledges without another authentication signature.
Write `{"refresh":true}` to `begin` to explicitly refresh those credentials.

This package implements the Polymarket Petal at `petals/polymarket/...` using the
`bloom:route@0.1.0` component ABI.

## Persona session accounting

Orders use complete ERC-7739 `TypedDataSign` EIP-712 data through the generic
`bloom.message.review` v1 format. Every signed field, including metadata and
builder, is included. The Petal verifies that the generic typed data produces
the exact protocol digest and verifies the returned signature against the owner.

BUY declares its six-decimal pUSD principal debit on Polygon using the exact
contract `0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb`. SELL leaves spend
unspecified and requires exact review. Onboarding approvals, funding,
withdrawals and other continuing permissions also retain exact review.
Operator trading fees remain bounded and disclosed in the local draft and
receipt; the signed order does not contain or enforce those fees.

A ClobAuth login explicitly declares `Spend::None`. With installer-certified
zero-spend provenance and an active Trader or Degen grant, it uses that grant's
rate cap and no token budget. Institutional requires exact review. Each refreshed
server timestamp is a distinct durable signing operation. Pending approval
retries keep the original timestamp and operation identity. Explicit refresh
advances the timestamp only after the previous attempt signed or completed.

Broker trusts the pinned, installer-signed package's declared effects. It binds
account, network, payload and grant, checks token policy and limits, and reserves
usage, without a Polymarket-specific verifier. A persona choice alone does not
activate a grant. Signed principal volume remains reserved after cancellation,
venue expiry or an unknown submission outcome; rearm preserves prior usage.

## Layout

- `petal-build.toml` pins the canonical Bloom Petal SDK and configures the
  canonical route builder.
- `route/files/` contains the route controllers. Each `*.rs` file maps to one
  Bloom route component and is compiled by the canonical `petal` CLI.
- `route/src/` contains Polymarket-specific domain code used by those route
  controllers: HTTP/store/signing infrastructure, onboarding, funding, trading,
  venue preferences, order construction, EIP-712, wallet calls, and typed DTOs.
- `scripts/build.sh` resolves the exact canonical builder revision and installs
  the complete generated route tree only after every component succeeds.

Directory endpoints use
`$index.rs`; `$list.rs` is intentionally unsupported because the Bloom Guest
world already has a separate `list` export.

## Runtime Boundaries

This petal owns the Polymarket behavior. It performs Polymarket HTTP calls
directly through the HTTP import, persists petal-owned state through the
private store import, uses signing intents for CLOB and relayer signatures,
reads mediated wallet/chain state through generic Bloom interfaces, and stages
funding through the generic EVM outbox. Enso credentials are provisioned
through the write-only `settings/enso-api-key` route and remain in the Petal
package-global secret store. The trusted Enso router value is also shared across accounts.

Per-account venue preferences are read at `settings/venue.toml` below the
selected account directory and stored in the Petal's own state. They are advisory application configuration only. The Petal
does not read or replace Broker/Signer-authoritative wallet policy, approval
budgets, or signing limits.

CLOB login uses the `polymarket.clob_auth.v1` class declared as zero spend in
this package's signing policy. The timestamp is refreshed by the Petal and the
complete payload is signed again. The host uses the same generic prepared-action
and grant rules as other packages. API credentials remain in the secret store.

The Petal is wallet-scoped and supports **numbered accounts**. `[wallet]` route
parameters are Bloom wallet ids, and the owner EOA is read from
`wallets/<wallet>/<account>/address.evm`. The retired wallet-root `address`,
`public_key`, and `addresses.json` leaves are never read. A missing address
leaf fails with an error that names the path.

It uses only the Petal route ABI and does not delegate to the legacy native
`polymarket/...` VFS handler.

Generated `petal/polymarket/**/*.wasm` files are ignored build output and should
not be committed.

Generate the route components with:

```sh
scripts/build.sh
```

Run tests with:

```sh
cargo test --manifest-path route/Cargo.toml
```

After changing `route/files/`, `route/src/`, or `petal-build.toml`, run both
commands before opening or updating a PR.

## Releases

Installable Petal archives are built and published by the reusable release
workflow in [`bloom-directory/petal`](https://github.com/bloom-directory/petal).
This repository owns only the thin tag-triggered caller in
`.github/workflows/release.yml`; it must pin that workflow to a full commit SHA.
The caller also passes that exact commit as `petal-tooling-ref`, so the workflow
implementation and the CLI that creates the package come from one reviewed
revision.

Release tags use Semantic Versioning with a `v` prefix. The first release that
publishes an installable archive is `v0.1.3`; the existing `v0.1.0` through
`v0.1.2` source tags remain unchanged. For a tag such as `v0.1.3`, the canonical
release files are:

- `polymarket-v0.1.3.petal.tar.gz`
- `SHA256SUMS`
- `petal-release.json`

The archive is platform-neutral: it contains WebAssembly route components and
Petal package metadata, not host-native executables. Bloom pins a release tag,
its resolved source commit, the asset filename, and its checksum; setup then
downloads and validates that immutable asset from this repository instead of
compiling the Petal locally.

To publish a release:

1. Ensure CI passes on `main` and choose the next Semantic Versioning tag.
2. Create the tag from the exact reviewed commit and push it.
3. Confirm the release workflow publishes all three files and that the source
   commit in `petal-release.json` matches the tagged commit.
4. Update Bloom's built-in Petal catalog in a separate reviewed change.

Do not retag or replace an existing release asset. Publish a new patch release
for packaging-only corrections.

## Account-scoped routes

Wallet features keep their root subtrees: `/petals/polymarket/{onboard,settings,account,positions,fund,trade,builder-keys,redeem,withdraw,revoke-approvals}/<wallet>/<index>/`. Obligations use `/petals/polymarket/obligations/<wallet>/<index>/status.json`.

`[wallet]` and adjacent `[index]` are explicit route captures. Bloom resolves them against the live core wallet projection and supplies trusted `bloom.wallet` and `bloom.account` context. Every numbered account, including 0, has a separate private store for account state. The manifest shares only the Enso API key and router through the package-global store. Legacy unnumbered settings and sessions are not carried into account 0. The core wallet tree remains `/wallets/<wallet>/<index>/`.


Before upgrading from routes without `[index]`, finish and reconcile pending operations using the installed build. Retain its package and private records until recovery is complete; do not delete them. A new route/package cannot inspect outbox entries staged by the old route/package. Core wallet custody and outbox entries remain intact. Modern numbered account stores are carried through signed package lineage; the legacy unnumbered store is not automatically imported.
Retain deposit-wallet onboarding identity, credentials, funding and relayer receipts, and trade records from the installed build until pending funds and positions have been reconciled. Externally funded deposit wallets and venue orders survive local route upgrades; use the old build to inspect their identity and recovery state.

### Prepared signing

Order signing persists a typed `PreparedOrder` before requesting a signature.
Its complete order, owner, funder, chain, market kind, draft and local
review-intent binding are checked again on every retry. The generic review
contains the full ERC-7739 `TypedDataSign` typed data; no protocol-specific
Broker witness is sent. Metadata and builder attribution must be zero.

The Petal owns venue submission and reconciliation. Stable operation keys
preserve signing retries; read-only status uses the separate signing-status
capability. This unreleased contract makes a clean break from prepared witness
records: use a new candidate package and fresh candidate state. A frozen draft
cannot be revalidated into a different order; reconcile it before creating a
replacement.
