# Native Feature Parity Implementation Plan

## Goal

Close the remaining native Polymarket feature gaps without delegating Petal
routes to Bloom's legacy `/polymarket` handler or adding Polymarket-specific
host IPC.

The change covers:

- configurable Gamma, Data API, CLOB, and relayer origins;
- configurable settlement chain name and chain ID;
- automatic, manual, and disabled relayer credential modes;
- write-only manual relayer credentials;
- legacy EOA credentials/read-only mode; and
- one bounded Sealed Approval ceremony for the complete, predetermined set of
  onboarding signatures.

## Design

### Trusted application configuration

Bloom configuration gains a generic per-application section:

```toml
[petals.apps.polymarket.endpoints]
gamma = "https://gamma-api.polymarket.com"
data = "https://data-api.polymarket.com"
clob = "https://clob.polymarket.com"
relayer = "https://relayer-v2.polymarket.com"

[petals.apps.polymarket.values]
chain = "polygon"
chain_id = "137"
```

Each `petal.toml` network rule may name an endpoint binding. Its packaged host,
methods, and paths remain the signed permission ceiling. An operator override
may replace only the HTTPS authority for that binding. It must not contain
userinfo, a path, query, or fragment, and must use the default HTTPS port.
There is no wildcard network permission and guest-writable storage cannot
change an authenticated request's destination.

The generic runtime environment exposes resolved application values to a route.
The Polymarket Petal uses those values for URL construction and chain routing.
Before using a configured chain, it verifies that `eth_chainId` for the named
Bloom chain equals the configured numeric chain ID.

### Petal-owned venue settings

Public venue behavior is stored in the Petal's private state namespace:

- `builder_key_mode`: `auto`, `manual`, or `disabled`;
- `legacy_eoa_mode`: boolean.

Manual relayer credentials are stored only in the secret namespace and are
exposed through a write-only setting. A complete manual key/address pair takes
precedence. Partial credentials fail closed. `manual` without a complete pair
and every relayer operation in `disabled` mode return actionable setup errors.

Legacy EOA mode creates/derives CLOB credentials for the owner EOA and supports
read-only account views. It must report the account as non-tradeable and reject
order posting, funding, relayer onboarding, redeem, approval revocation, and
pUSD withdrawal before any signing or network side effect.

### Bounded multi-signature approval

The generic Petal signing interface gains an additive batch operation. A batch
contains an ordered list of wallet, 32-byte hash, and intent tuples. Bloom:

1. validates trusted package and route provenance;
2. seals the complete ordered request set and its digest into one action;
3. fixes the grant signature allowance to the request count;
4. refuses duplicate, empty, oversized, reordered, or altered batches;
5. returns one approval challenge when no matching grant exists; and
6. after approval, validates a hash-specific attestation for every entry before
   producing any signatures.

Onboarding prepares both the CLOB authentication hash and relayer batch hash
before requesting approval, persists their review material, and retries the
exact same batch. No new hash may be added after approval.

## Work Breakdown

1. Extend Bloom's generic Petal config, manifest network rules, runner options,
   environment binding, and exact-host policy resolution.
2. Replace Polymarket constants at request sites with resolved endpoint and
   chain settings; remove hard-coded `chains/polygon` use.
3. Add Petal settings routes and typed validation for relayer and legacy modes.
4. Route relayer authentication through a single credential selector shared by
   onboarding and later relayer actions.
5. Add the generic batch-sign ABI, VM adapter, daemon action construction,
   sealed terms, attestation validation, and audit events.
6. Change onboarding to prepare and consume the bounded signing batch while
   preserving stable retry artifacts.

## Pragmatic Test Coverage

Tests focus on security and behavior boundaries:

- one manifest/policy test proves a named override allows the configured exact
  host while the packaged method/path ceiling and unrelated hosts remain denied;
- one config/runtime test covers endpoint and chain value injection plus invalid
  origin rejection;
- table tests cover relayer precedence, partial credentials, and all three
  modes without exercising HTTP repeatedly;
- one legacy EOA workflow test proves credentials/read status and one common
  guard test covers every forbidden value-moving route;
- host tests prove an approved batch signs the exact set once and rejects
  mutation, reordering, replay, and excess size;
- host-boundary tests prove approval-required then retry produces the exact
  ordered signatures, while a focused Petal test proves the persisted relayer
  artifact expires at its protocol deadline. The built 94-route package is
  then validated against Bloom's exact imported-interface shapes.

Existing protocol-vector tests are reused. Endpoint permutations and every
route/mode combination are not duplicated when they share the same selector or
guard.

## Verification

- focused Bloom tests for `bloom-proto`, `bloom-petals`, `bloom-auth-api`,
  `bloom-keystore`, and `bloom-daemon` as touched;
- `cargo test --manifest-path route/Cargo.toml`;
- `scripts/build.sh`; and
- a final scan confirming the Petal does not read or write native
  `polymarket/...` VFS routes.
