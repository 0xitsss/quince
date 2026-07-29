<!--
SPDX-FileCopyrightText: 2026 0xitsss

SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
-->

# Production beta runbook

This runbook is the operational path for a **single-operator, single-symbol
production beta**. It is deliberately a promotion checklist, not a promise of
profitability. A strategy advances only after the previous gate has produced
evidence that can be inspected and reproduced.

The supported progression is:

```text
offline preflight → public + shadow → replay research → Binance Futures testnet
→ limited Binance live beta
```

Hyperliquid is supported for wallet onboarding and public market data. Its
authenticated execution path is currently fail-closed; it is **not** a live
beta venue until the binary explicitly enables it. Do not treat a configured
wallet as permission to trade.

## Non-negotiable rules

- Use a dedicated account or wallet with funds you can afford to lose. Never
  reuse a personal wallet or a general-purpose exchange API key.
- Never put a private key, API secret, recovery phrase, or full credentials in
  QFL, Git, shell history, a `.env` file committed to Git, dashboard requests,
  or a support message.
- The default dashboard is loopback-only and read-only. It is visibility, not
  remote order control.
- Set explicit small limits for every beta process. Defaults are safety bounds,
  not an approval for a given amount of capital.
- One process, one strategy revision, one symbol, and one operator at a time.
  Stop and reconcile before restart after a crash or an uncertain order state.

## 0. Build and offline preflight

Build the pinned toolchain, then validate the exact configuration without
opening an exchange socket, loading credentials, or creating order artifacts.

```bash
cargo +nightly build --locked

QUINCE_PUBLIC=1 \
QUINCE_SHADOW=1 \
QUINCE_STRATEGY=strategies/scalper.qfl \
QUINCE_SYMBOL=btcusdt \
QUINCE_MAX_POSITION=0.001 \
QUINCE_MAX_ORDER_NOTIONAL=25 \
QUINCE_MAX_POSITION_NOTIONAL=50 \
QUINCE_MAX_DRAWDOWN=0.02 \
QUINCE_MAX_DAILY_LOSS=10 \
QUINCE_MAX_ORDER_FREQ=2 \
QUINCE_MAX_MARKET_DATA_AGE_MS=2000 \
cargo run --locked --bin quince -- preflight
```

The command must print JSON with `"status":"ok"`, the intended exchange and
network, `"input_mode":"public"`, and `"execution_mode":"shadow"`. Correct
the configuration rather than weakening a limit to make preflight pass.

Before every promotion, verify that the prior process left no ambiguous orders:

```bash
cargo run --locked --bin quince -- journal verify trades.orders.jsonl
```

If it reports unresolved client order IDs, **do not restart**. Reconcile every
listed ID against the exchange first.

## 1. Dedicated Hyperliquid wallet (public-data use)

The initial interactive launch offers wallet creation. To make this explicit,
run the wizard in a private terminal:

```bash
QUINCE_WALLET_SETUP=1 cargo run --locked --bin quince
```

Choose *create* for a new dedicated wallet, or import only a dedicated key.
The private key is stored in `wallet.enc.json` with AES-256-CBC plus
encrypt-then-MAC authentication; the passphrase is never stored and the public
profile contains only the address. For a non-interactive authenticated process,
inject `QUINCE_WALLET_PASSPHRASE` from a secret manager. The wizard must never
be run through screen sharing, copied terminal transcripts, or a shell command
containing the private key or passphrase.

This is not required for Binance public/shadow/replay work. It is required
before any future authenticated Hyperliquid integration, which Quince does not
currently enable.

## 2. Public data in shadow mode

Shadow mode evaluates the strategy but suppresses each order before journal and
exchange dispatch. Run it long enough to observe normal market conditions,
quiet periods, reconnects, and at least one planned restart.

```bash
QUINCE_PUBLIC=1 \
QUINCE_SHADOW=1 \
QUINCE_DASHBOARD=1 \
QUINCE_STRATEGY=strategies/scalper.qfl \
QUINCE_SYMBOL=btcusdt \
QUINCE_MAX_POSITION=0.001 \
QUINCE_MAX_ORDER_NOTIONAL=25 \
QUINCE_MAX_POSITION_NOTIONAL=50 \
QUINCE_MAX_DRAWDOWN=0.02 \
QUINCE_MAX_DAILY_LOSS=10 \
QUINCE_MAX_ORDER_FREQ=2 \
QUINCE_MAX_MARKET_DATA_AGE_MS=2000 \
cargo run --locked --bin quince
```

Inspect the loopback dashboard at `http://127.0.0.1:3000`. `GET /healthz`
only proves the dashboard process can respond. `GET /readyz` is the stricter
signal: it requires a fresh healthy journal snapshot, no unresolved orders,
and `execution_sync_ready=true`.

For Hyperliquid public testnet observation, use the strategy directives:

```bash
QUINCE_PUBLIC=1 \
QUINCE_SHADOW=1 \
QUINCE_STRATEGY=strategies/hyperliquid_public.qfl \
QUINCE_SYMBOL=BTC \
cargo run --locked --bin quince
```

Promote only if telemetry stays healthy: no unexplained stream-integrity
growth, no stale-data latch, no unresolved journal IDs, and strategy behavior
matches the expected signal logic. A dashboard green light is necessary but is
not a trading recommendation.

## 3. Replay research gate

Collect or import a capture, then use explicit cost assumptions. The report is
deterministic for the same strategies, capture, symbol, and assumptions.

```bash
QUINCE_SYMBOL=BTCUSDT \
QUINCE_REPLAY_FEE_BPS=4 \
QUINCE_REPLAY_SLIPPAGE_BPS=2 \
QUINCE_REPLAY_INITIAL_EQUITY=10000 \
cargo run --locked --bin quince -- research \
  strategies captures/btcusdt.jsonl target/research/btcusdt
```

Review both `target/research/btcusdt/research-report.html` and the paired
machine-readable `research-report.json`. Sharpe and Sortino in this report are
per-observation, not annualized. Do not annualize irregular tick events by
hand. Require an out-of-sample capture and reject a candidate if its result is
dependent on a single session, unrealistic fees, or zero slippage.

## 4. Binance Futures testnet gate

Create a dedicated **Binance Futures testnet** API key with only the minimum
permissions the venue requires. Keep the credential outside the repository and
inject it through your local secret manager or CI secret facility. Start in
shadow mode first, even on testnet:

```bash
# BINANCE_API_KEY and BINANCE_SECRET_KEY are already injected by your
# local secret manager; do not type their values in this terminal command.
QUINCE_TESTNET=1 \
QUINCE_SHADOW=1 \
QUINCE_STRATEGY=strategies/scalper.qfl \
QUINCE_SYMBOL=btcusdt \
QUINCE_MAX_POSITION=0.001 \
QUINCE_MAX_ORDER_NOTIONAL=25 \
QUINCE_MAX_POSITION_NOTIONAL=50 \
QUINCE_MAX_DRAWDOWN=0.02 \
QUINCE_MAX_DAILY_LOSS=10 \
QUINCE_MAX_ORDER_FREQ=2 \
QUINCE_MAX_MARKET_DATA_AGE_MS=2000 \
cargo run --locked --bin quince -- preflight
```

After preflight and a clean shadow observation window, remove only
`QUINCE_SHADOW=1` to exercise testnet orders. Verify each submitted order,
fill, cancellation, and restart against the venue, then run journal verification
again. Do not skip directly from public data to mainnet.

## 5. Limited Binance live beta

Live mode is an explicit Binance-only boundary. Hyperliquid authenticated
execution remains unavailable. Before a live start, require all of the
following:

1. A fresh successful preflight for the exact strategy revision and symbol.
2. A clean journal with no unresolved IDs.
3. Public/shadow and testnet evidence for the same strategy parameters.
4. A dedicated mainnet API key with the least privileges possible and IP
   restrictions configured at the venue.
5. An operator present for the entire initial session and a written maximum
   loss that is lower than the account balance.

Use explicitly small bounds. These are examples, not recommended amounts:

```bash
# BINANCE_API_KEY and BINANCE_SECRET_KEY are already injected by your
# local secret manager; do not type their values in this terminal command.
QUINCE_LIVE=1 \
QUINCE_DASHBOARD=1 \
QUINCE_STRATEGY=strategies/scalper.qfl \
QUINCE_SYMBOL=btcusdt \
QUINCE_MAX_POSITION=0.001 \
QUINCE_MAX_ORDER_NOTIONAL=25 \
QUINCE_MAX_POSITION_NOTIONAL=50 \
QUINCE_MAX_DRAWDOWN=0.02 \
QUINCE_MAX_DAILY_LOSS=10 \
QUINCE_MAX_ORDER_FREQ=2 \
QUINCE_MAX_MARKET_DATA_AGE_MS=2000 \
cargo run --locked --bin quince
```

The engine fail-closes on missing/failing synchronization, stale or invalid
market data, risk breaches, and reconciliation failure. These controls reduce
risk; they cannot eliminate exchange, software, network, or market risk.

## Emergency stop and recovery

There is no default HTTP endpoint that can be exposed remotely to cancel
orders. For an immediate stop:

1. Interrupt the Quince process in its controlling terminal (`Ctrl-C`). This
   stops new local order submission; it does not cancel orders already accepted
   by the venue.
2. Use the exchange's authenticated UI or its established emergency procedure
   to cancel open orders and, if needed, flatten the position. Verify the
   resulting account state there.
3. Disable or revoke the dedicated API key at the venue if credentials may be
   compromised.
4. Do **not** restart Quince yet. Inspect the local journal, reconcile every
   client order ID with the venue, and only then verify it:

   ```bash
   cargo run --locked --bin quince -- journal inspect trades.orders.jsonl
   cargo run --locked --bin quince -- journal verify trades.orders.jsonl
   ```

5. Preserve the journal and logs for incident review. Start the next session
   in `QUINCE_PUBLIC=1 QUINCE_SHADOW=1` until the cause is understood.

The internal control plane is bounded and audited, but its HTTP transport is
not enabled by the default dashboard. Do not rely on an unexposed endpoint as
an emergency mechanism.
