# Polymarket Petal

This package implements the Polymarket Petal at `petals/polymarket/...` using the
`bloom:route@0.1.0` component ABI.

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

The route tree currently builds 113 route components. Directory endpoints use
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

## Default builder code

Polymarket's [Builder Program](https://docs.polymarket.com/programs/builders/overview)
assigns a `bytes32` builder attribution code (not an address) when you
register a Builder Profile and set its fee rates on polymarket.com; every
order this Petal signs carries that code in the CTF Exchange order struct's
`builder` field so Polymarket can attribute volume and revenue share to it.
Fee rates and payout are managed entirely by Polymarket against that
registration — there is no on-chain approval step.

A release build embeds Bloom's own code so orders carry it without an operator
having to configure one. This is public on-chain data, not a credential — it
is serialized into every `OrderFilled` event and builder profiles are publicly
queryable — but it is injected at build time rather than committed, because
this repository is public. The `POLYMARKET_BUILDER_CODE` repository secret is
passed to the pinned release workflow as its generic `compile-time-secret`
input, which `scripts/build.sh` re-exports for
`EMBEDDED_DEFAULT_BUILDER_CODE` in `route/src/builder_code.rs`.

The cost is that a tagged release cannot be rebuilt byte for byte from source
alone. What compensates is that the code is bound into the approval the owner
signs, so an order cannot carry a code the owner did not approve. A build that
sets nothing has no default, and orders carry a zero builder field exactly as
they did before this was wired up.

A release build therefore resolves a builder code on every order, which means
`settings/builder-fee-ceiling` must also be written before any order can be
placed: a configured code owes a fee, the fee-free route refuses it, and the
fee-bearing route refuses a code with no ceiling. Both routes fail closed with
the setting to write named in the error. One write recovers.

**A buy cannot be posted on the fee-bearing class.** That class exists to
declare a maximum fee, and on a buy there is no maximum to declare. The signed
CLOB V2 order carries no fee field — V2 removed `feeRateBps` — so the signature
commits to `makerAmount` and `takerAmount` and says nothing about a fee, while
Polymarket's operator supplies the fee when it fills the order. Neither
remaining bound helps: the exchange's on-chain maximum-fee check is disabled on
both deployments, and onboarding grants the exchange `U256::MAX` pUSD, so what
it may pull is capped by the wallet balance and nothing else. A review
reproduced a signed 50 pUSD buy settling for 550 — 50 principal and 500 fee.

Declaring a finite fee there would hand Broker a value limit the same signature
permits exceeding, and state a total to the owner that is not one, so
`post_builder_code` refuses a buy outright. A sell is unaffected: what leaves
the wallet is the shares in `makerAmount`, which the signature does bound, and
a fee can only reduce the proceeds the sale returns.

The consequence is worth stating plainly: while a builder code that charges on
the paying side is configured, **buys cannot be placed at all** — the
fee-bearing route refuses them for the reason above and the fee-free route
refuses an order whose code would charge it. Restoring them needs a bound
something enforces: the exchange's maximum-fee check enabled, or an allowance
set to the order's intended maximum instead of `U256::MAX`.

A ceiling may be zero on one side. When it is zero on the side an order pays —
a maker-only builder taking a FAK or FOK order — that order owes nothing, so it
posts through `post`, unattributed and carrying no code. The fee-bearing route
will not declare a zero fee, and refusing the order on both routes would leave
such a builder unable to trade at all.

`settings/builder-code` writes an operator override (any hex string up to 32
bytes, with or without `0x`) that takes precedence over the release default,
or clear it with an empty body; `settings/builder-code-status.json` reports
which one (if either) is currently in effect.

Both that code and its ceiling decide who is paid and how much, and both are
ordinary agent writes.

**Known gap, tracked as a pre-merge gate.** Everything below about what the
ceremony shows the owner describes advisory text that, at the pinned Bloom and
Broker revisions, reaches no screen. `bloom-daemon` reduces a Petal advisory to
`advisory_digest`, `sign_or_prepare_petal` in `bloom-vfs::exact_signing` takes
no advisory parameter, and the pinned `bloom-broker-api` has no field for one;
the Broker ceremony's `attributed_advisory_items` is populated nowhere outside
its own tests. The digest is bound into the signing identity, so changing the
advisory invalidates an approval — it is committed and not displayed, which is
exactly why it reads like a working control.

The advisory content below is therefore necessary but not yet sufficient. What
bounds an agent today is the Broker's approved-capacity limits, the declared
fee under a fee-bearing class, and the Petal-side consistency checks in the
next section — not anything the owner reads at approval time.

**What bounds that is the owner's per-order ceremony, and only that.** The
Machine is inside the threat model — the Sealed Approvals architecture says in
as many words that a compromised Petal or Machine may lie within the claim's
trust model — and every store object the flow reads is a file that Machine can
write. So the ceremony states the trade as well as its cost: side, size,
market, outcome and limit price alongside the builder code, the rate, the fee
and what the total covers. It used to name the fee and nothing else, which made
two orders differing in every economic respect but equal in notional produce
byte-identical approval text. Venue-supplied text in that advisory is stripped
of control characters and quotes and truncated, so a market title cannot forge
a line of it.

What the Petal adds on top is consistency, so an honest flow cannot drift and a
dishonest one has to be deliberate. The signed order is pinned to the draft the
owner reviewed — token, amounts, side, maker wallet and signature type — which
is what stops a resumed post naming a different trade; a fee-free order may not
carry a builder code at all, since its ceremony declares no fee and names no
beneficiary; the review intent names the code and the fee, so a change between
the agent's revalidation and the signature fails the comparison; and a resumed
builder-coded post reads its fee back from the artifact rather than recomputing
it. The digest cross-check between the reconstructed order and the signed hash
catches corruption, not an adversary: a preimage and its hash are one object.

Changes to either setting are appended to a package-global record, sequenced,
bounded, and reported as `recent_changes` by
`settings/builder-code-status.json`. It is an operational record, **not** a
tamper-evident one: anything that can write the settings can write the record.
It is reported as `null` rather than empty when it cannot be read, so a lost
history does not read as a quiet one, and the sequence numbers mean a tail that
does not begin at 1 says how many changes preceded it.

## Account-scoped routes

Wallet features keep their root subtrees: `/petals/polymarket/{onboard,settings,account,positions,fund,trade,builder-keys,redeem,withdraw,revoke-approvals}/<wallet>/<index>/`. Obligations use `/petals/polymarket/obligations/<wallet>/<index>/status.json`.

`[wallet]` and adjacent `[index]` are explicit route captures. Bloom resolves them against the live core wallet projection and supplies trusted `bloom.wallet` and `bloom.account` context. Every numbered account, including 0, has a separate private store for account state. The manifest shares only the Enso API key, router, and public builder-attribution override through the package-global store. Legacy unnumbered settings and sessions are not carried into account 0. The core wallet tree remains `/wallets/<wallet>/<index>/`.


Before upgrading from routes without `[index]`, finish and reconcile pending operations using the installed build. Retain its package and private records until recovery is complete; do not delete them. A new route/package cannot inspect outbox entries staged by the old route/package. Core wallet custody and outbox entries remain intact. Modern numbered account stores are carried through signed package lineage; the legacy unnumbered store is not automatically imported.
Retain deposit-wallet onboarding identity, credentials, funding and relayer receipts, and trade records from the installed build until pending funds and positions have been reconciled. Externally funded deposit wallets and venue orders survive local route upgrades; use the old build to inspect their identity and recovery state.
