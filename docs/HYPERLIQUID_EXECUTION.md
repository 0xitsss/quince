# Hyperliquid Execution Acceptance Specification

> **Status:** design and acceptance contract. Hyperliquid mutations remain
> disabled until every required acceptance criterion below passes on testnet.
>
> This document deliberately separates protocol facts verified against
> Hyperliquid-controlled sources from implementation decisions and open
> questions. It is not trading advice.

## Scope and safety boundary

This specification covers native Hyperliquid L1 trading actions only:

- order placement;
- cancel by exchange order ID and by client order ID (`cloid`);
- account/order reconciliation after an ambiguous result.

Transfers, withdrawals, agent approval, vault administration, multi-sig, and
other **user-signed actions** are out of scope. They use a different EIP-712
scheme and must not be enabled as an incidental consequence of implementing
orders.

The implementation must fail closed. A timeout, HTTP/WS disconnect, malformed
response, signature mismatch, nonce uncertainty, or reconciliation failure is
an `SubmissionUnknown`/incident state—not permission to retry the action.

## Verified protocol facts

The following are directly verified from official Hyperliquid documentation or
its official SDK source as linked below.

### Endpoint and order wire format

- Submit L1 actions to `POST /exchange`: mainnet
  `https://api.hyperliquid.xyz/exchange`, testnet
  `https://api.hyperliquid-testnet.xyz/exchange`.
- An order action has `type: "order"`, `orders`, and `grouping`. An order wire
  uses `a` (asset), `b` (buy), `p` (price string), `s` (size string), `r`
  (reduce-only), `t` (limit/trigger), and optional `c` (cloid).
- A cloid is an optional 128-bit hex string. It is the primary recovery key for
  Quince-originated orders; generate it before signing and persist it in the
  order journal.
- Limit time-in-force is one of `Alo`, `Ioc`, or `Gtc`. `Alo` is post-only;
  `Ioc` cancels unfilled remainder; `Gtc` rests normally.
- The outer request contains `action`, `nonce`, `signature`, optional
  `vaultAddress`, and optional `expiresAfter` (milliseconds).

Sources: [exchange endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint),
[official Python SDK signing source](https://raw.githubusercontent.com/hyperliquid-dex/hyperliquid-python-sdk/master/hyperliquid/utils/signing.py).

### Canonical L1 action hash and signature

For trading actions, Quince must reproduce the official SDK algorithm byte for
byte. It must **not** sign JSON text, a serde representation, or only an
`action_hash` as though it were a generic ECDSA digest.

1. Serialize the complete action object using canonical MessagePack compatible
   with the official SDK's `msgpack.packb(action)`.
2. Append `nonce` as exactly eight unsigned big-endian bytes.
3. Append the vault selector:
   - no vault/subaccount: one byte `0x00`;
   - vault/subaccount: `0x01`, then the raw 20 bytes of the EVM address.
4. If `expiresAfter` is present, append `0x00` followed by its eight-byte
   unsigned big-endian value. If absent, append nothing further.
5. Keccak-256 the resulting bytes. This 32-byte value is the connection ID.
6. EIP-712-sign an `Agent` message, not the action directly:

   ```text
   domain.name              = "Exchange"
   domain.version           = "1"
   domain.chainId           = 1337
   domain.verifyingContract = 0x0000000000000000000000000000000000000000

   Agent {
     string source;       // "a" mainnet, "b" testnet
     bytes32 connectionId;
   }
   ```

7. Submit `{ r, s, v }` from that EIP-712 signature. `v` must be the format
   accepted by the official SDK (`27` or `28`); do not silently normalize an
   unverified recovery ID at the transport boundary.

The source byte is a replay boundary: `"a"` for mainnet and `"b"` for testnet.
The EIP-712 chain ID for this L1 `Agent` signature is always `1337`; it is not
the wallet's selected EVM chain. The exact algorithm above is visible in the
official SDK's `action_hash`, `construct_phantom_agent`, `l1_payload`, and
`sign_l1_action` functions.

Sources: [official signing guide](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/signing),
[official Python SDK implementation, lines 161–227](https://raw.githubusercontent.com/hyperliquid-dex/hyperliquid-python-sdk/master/hyperliquid/utils/signing.py),
[official Rust SDK repository](https://github.com/hyperliquid-dex/hyperliquid-rust-sdk).

### Nonces, API wallets, vaults, and expiry

- The exchange recommends a current timestamp in milliseconds for the nonce.
- Hyperliquid stores the 100 highest nonces per address. A nonce may be
  invalidated deliberately with the `noop` L1 action; it is not safe to assume
  monotonically consecutive nonce semantics like Ethereum transactions.
- A user, vault, or subaccount sharing one API wallet also shares that API
  wallet's nonce set. Quince therefore needs one durable nonce coordinator per
  signer address, rather than one counter per symbol or strategy.
- To act for a vault/subaccount, signing is performed by the master account and
  the target on-chain address is sent as `vaultAddress`. This is not enabled in
  Quince's first execution phase; signer/account equality remains mandatory.
- `expiresAfter` rejects stale L1 actions after its millisecond timestamp.
  Rejection caused by stale expiry costs five times the normal address-based
  rate limit, so it must be set with a bounded clock-skew policy rather than a
  near-zero deadline.
- API/agent wallets are approved separately by a user-signed `approveAgent`
  action. The official SDK requires the main wallet public address as the
  account address even when an API wallet holds the signing private key.

Sources: [nonces and API wallets](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/nonces-and-api-wallets),
[exchange endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint),
[official Python SDK README](https://github.com/hyperliquid-dex/hyperliquid-python-sdk).

## Repository gap: what must land before enabling execution

This is a repository-specific audit of the current boundary, not a protocol
claim.

1. `exchange/src/hyperliquid/execution.rs` intentionally has no canonical
   MessagePack encoder, Keccak-256 implementation, EIP-712 `Agent` encoder, or
   HTTP submit path. Its current `HyperliquidSigner::sign_l1_action` takes only
   a 32-byte action hash, while protocol correctness also requires construction
   of—and signature over—the fully specified EIP-712 `Agent` payload.
2. `exchange/Cargo.toml` has no pinned MessagePack and EIP-712 implementation,
   and no vendored official SDK. Adding a generic signing crate alone is not an
   acceptance criterion: canonical map ordering/field omission and EIP-712
   typed-data bytes must prove parity with the official implementation.
3. The official docs point to official Python and Rust SDKs, but the public API
   docs do not publish a stable, standalone corpus of golden L1 signing
   vectors. Quince must therefore add checked-in test fixtures generated from a
   **pinned commit or released version** of an official SDK, recording the
   action JSON, nonce, vault address, expiry, network, action hash, EIP-712
   digest, signer address, and `{r,s,v}`. Fixture provenance must include the
   SDK repository URL and immutable commit SHA.
4. Do not use live exchange acceptance as the only signature oracle. Offline
   fixtures must fail on any byte-level incompatibility before a network test
   starts.

### Required fixture matrix

Generate the following solely with a fixed official SDK version, review the
fixture diff, then commit it. Do not generate expected values with Quince.

| Case | Required variation |
| --- | --- |
| Limit GTC order | no vault, no expiry, mainnet source `a` |
| Limit ALO order | testnet source `b` |
| IOC order | distinct nonce and a price/size requiring wire-string normalization |
| Batch order | more than one order, stable array order |
| Cancel by oid | no vault |
| Cancel by cloid | exact 128-bit cloid |
| Expiring action | `expiresAfter` included in hash |
| Vault/subaccount hash | master signer plus target vault address; fixture only until delegation is enabled |
| Negative controls | altered nonce, network source, expiry, vault selector, field order, and one signature byte must not recover the expected signer |

The Quince implementation must exactly match the action hash, final EIP-712
digest, `r`, `s`, `v`, and recovered EVM address for every positive fixture.

## Testnet enablement criteria

All local unit/integration tests run without secrets. The network suite is
explicitly opt-in, uses a dedicated funded testnet account/API wallet, has an
absolute maximum notional configured outside the repository, and never points
at mainnet.

1. **Metadata and precision.** Fetch the official metadata before creating any
   order; resolve asset index and enforce its size decimals locally. A stale or
   missing mapping rejects the intent.
2. **Signature parity.** Pass every offline fixture above before the testnet
   suite is permitted to load a key.
3. **Signer identity.** Recover the `Agent` signature locally and verify that
   it equals the configured API wallet. Verify that the configured account is
   the intended master account. Reject a signer/account mismatch.
4. **Happy path.** Submit a small resting testnet `Alo` order with a unique
   cloid. Require a successful response containing a resting exchange order
   ID, then query open orders and find the same order by cloid and ID.
5. **Cancel path.** Cancel the known resting order by cloid. Re-query open
   orders until it is absent or reports a terminal cancellation. A local write
   or HTTP 200 alone is insufficient evidence.
6. **Ambiguous submission.** Inject a transport failure after the request is
   handed to the HTTP layer. Do not send another placement. Reconcile by cloid
   and classify exactly one of: known live/filled order, authoritative absence,
   or unresolved incident. The unresolved case blocks subsequent mutation.
7. **Nonce and expiry.** Test a duplicate nonce rejection and a deliberately
   expired action rejection. Ensure neither is retried automatically. Measure
   and log observed clock skew; block signing when it exceeds the configured
   safety budget.
8. **Restart recovery.** Kill the process after journaling a submission but
   before observing its response. On restart, load the journal, reconcile all
   non-terminal cloids, and refuse a new order until reconciliation completes.
9. **Rate-limit/backoff.** Respect documented endpoint weights; on a rate-limit
   response, stop new mutation and retain outstanding exposure. No blind
   high-frequency retry loop.
10. **Dead-man control.** Exercise `scheduleCancel` only in its own opt-in
    test. Verify scheduled cancellation in exchange state and document the
    maximum ten triggers/day and five-second minimum scheduling delay.

Sources: [info endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint),
[rate limits and user limits](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/rate-limits-and-user-limits),
[exchange endpoint: schedule cancel](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint).

## Reconciliation and incident contract

### Authoritative state to query

After any ambiguous or restart scenario, query the configured account's:

- open orders (match both exchange order ID and cloid where available);
- order status/history for every locally known exchange order ID;
- clearinghouse/account state for position and margin reconciliation;
- user event stream after reconnect, with a REST snapshot used to close any
  event gap.

The official SDK exposes `open_orders` and `user_state`/clearinghouse state
over the `info` endpoint, and its examples subscribe to `userEvents` while also
polling state. Quince must treat WebSocket delivery as a low-latency signal,
not as the exclusive recovery source.

Sources: [info endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint),
[official basic adding example](https://github.com/hyperliquid-dex/hyperliquid-python-sdk/blob/master/examples/basic_adding.py).

### Incident states and operator actions

| Condition | Engine state | Mandatory action |
| --- | --- | --- |
| Timeout/disconnect after write | `SubmissionUnknown` | Stop retries; reconcile by cloid and order state. |
| Unknown order discovered | `ExternalOrder` incident | Freeze new mutation; operator decides whether to cancel/adopt it. |
| Local/exchange quantity or position mismatch | `ReconciliationMismatch` | Freeze symbol/account; snapshot evidence; resolve before resume. |
| Invalid/duplicate nonce or expiry rejection | `NonceIncident` | Do not re-sign blindly; allocate a fresh coordinated nonce only after state review. |
| Signature recovery mismatch | `SigningIncident` | Disable signer immediately; preserve action bytes and fixture IDs; do not submit again. |
| Rate limit/backoff event | `Degraded` | Retain risk exposure, throttle mutation, and alert. |

For every incident, journal: immutable action bytes/hash, nonce, expiry,
network, signer/account addresses, cloid, request timestamp, transport result,
raw exchange response (redacted), and reconciliation snapshots. Never journal
the private key or an unredacted signature if the operational threat model
classifies it as sensitive.

## Implementation sequencing

1. Add a pure `HyperliquidL1Codec` with no HTTP or key access. Implement the
   canonical action bytes, action hash, `Agent` typed data, signature recovery,
   and fixture tests.
2. Change the signer boundary so a signer signs the exact EIP-712 `Agent`
   payload/digest, or keep typed-data construction in a reviewed adapter and
   expose the final payload for audit. A raw generic `action_hash` interface is
   insufficient by itself.
3. Add metadata/precision resolution and a durable per-signer nonce
   coordinator, then the HTTP submit client with no automatic retry.
4. Add REST+WebSocket reconciliation and journal recovery.
5. Run the opt-in testnet suite repeatedly, capture evidence, and conduct a
   security review. Only then consider enabling a separately gated testnet
   execution flag. Mainnet requires a new review and explicit operator opt-in.

## Open questions — do not assume

These items were not promoted to verified requirements because the linked
official API material does not give Quince a stable enough answer on its own:

1. Which official SDK release/commit will be pinned as fixture authority, and
   whether Quince should consume the official Rust SDK directly or independently
   implement the codec with parity fixtures. The initial recommendation is an
   independent minimal codec plus fixtures from both official SDKs where their
   outputs agree; adding an SDK dependency does not remove the need for
   regression fixtures.
2. The production policy for API-wallet rotation, revocation, valid-until names,
   and separate signer processes. These are operational controls beyond the
   basic order protocol.
3. Exact REST retention/history behavior required to prove authoritative
   *absence* of a timed-out cloid. Treat absence as unresolved until confirmed
   by a documented query and a bounded observation window.
4. Whether and when to support vaults, subaccounts, HIP-3 DEXs, multi-sig,
   triggers, or builder fees. Each changes action encoding, authorization, or
   reconciliation and needs its own acceptance specification.
