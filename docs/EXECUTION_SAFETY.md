# Execution Safety Contract

Quince treats an order as risk-visible until an authoritative exchange state
proves otherwise. A successful local write, WebSocket send, or cancellation
request is not proof that an order cannot fill.

## Current guarantees

### Shared lifecycle

The engine creates a process-unique client order ID before it submits an
order. The ID travels in `OrderRequest`, so an exchange adapter can use its
native idempotency field and look the order up after a lost response.

The in-memory lifecycle is:

```text
Waiting → Placed → PartiallyFilled → Filled
   │         │              │
   │         └──────────────┴→ CancelRequested → Cancelled
   └→ SubmissionUnknown ──(client-ID reconciliation)──→ Placed
```

- `SubmissionUnknown` keeps the full pending exposure in risk checks.
- `CancelRequested` keeps the remaining exposure until `order_status` reports
  a terminal state.
- Late fills and invalid quantities/prices cannot revive terminal orders.
- A transport timeout is never retried automatically.

## Binance Futures

Authenticated Binance execution currently uses the Futures WebSocket API.

- Binance mainnet requires `QUINCE_LIVE=1` in addition to credentials.
- Testnet is selected with `@network testnet` or `QUINCE_TESTNET=1`.
- Every submitted order includes the engine client ID as
  `newClientOrderId`.
- A timed-out/disconnected submission must be reconciled through
  `origClientOrderId` before anyone retries it.
- Limit orders are explicit `GTC`; market orders with a limit price are
  rejected locally.

Before production use, configure Binance symbol filters/precision and test
against a dedicated testnet account. User-data streams, reconnect recovery and
a durable order journal are still required for a complete production claim.

## Hyperliquid

The Hyperliquid public adapter provides market data. The authenticated adapter
boundary validates signer/account matching and order intent, but deliberately
does not submit orders yet.

Hyperliquid L1 actions require protocol-specific canonical encoding and EIP-712
signing. Quince will keep all mutations disabled until the implementation is
covered by official signing vectors, a reviewed submit path, account/order
reconciliation, and testnet integration tests.

## Operator checklist

1. Start with `QUINCE_MOCK=1`.
2. Verify a public market-data session with `QUINCE_PUBLIC=1`.
3. Configure a separate Binance testnet account; never reuse a broad-permission
   mainnet key for development.
4. Set explicit risk limits: position, daily loss, drawdown and order rate.
5. Test cancellation, disconnect, partial fill and restart recovery before
   considering any mainnet session.
6. Treat a `SubmissionUnknown` log as an incident: reconcile the client ID at
   the exchange before sending another order.

## Non-goals in the current release

- Guaranteed exactly-once execution across a process restart.
- Automatic retry after an ambiguous network result.
- Hyperliquid live order placement.
- Financial advice or profitability claims.
