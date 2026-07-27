# Quince: Build in Public

This is the operating plan for communicating about Quince in public. The
project is execution infrastructure, not a profit-promise or a signal service.

## Positioning

> Quince is a Rust-native runtime for systematic trading: strategies written in
> QFL compile to a compact VM, run behind explicit risk controls, and connect to
> exchange market data through adapters that fail closed when execution is not
> verified.

Use **"low-latency trading runtime"** and **"systematic trading
infrastructure"**. Do not lead with "HFT" until an end-to-end latency
methodology on production-like hardware is published.

Never claim profitability, guaranteed performance, live-execution support that
does not exist, or that Quince is financial advice.

## Accounts and handles

Use the same avatar, mint/teal visual language, bio, and `quince` link
everywhere. Create the accounts in this order.

| Priority | Platform | Account / handle | Role |
| --- | --- | --- | --- |
| 1 | GitHub | `0xitsss` | Founder identity and source of truth |
| 1 | GitHub Organization | `quince` if available; otherwise `quince-dev` | Project home if the project becomes multi-contributor |
| 1 | X | `@quincehq`; fallbacks: `@quince_dev`, `@quinceengine` | Official project updates |
| 1 | X | `@0xitsss` | Founder voice, design and engineering notes |
| 2 | Reddit | `u/0xitsss` | Technical discussion; do not create a brand account first |
| 2 | Discord | `Quince` server; invite `discord.gg/quince` only if available | Support once people actually ask for it |
| 3 | Farcaster | `@quince`; fallback `@quincehq` | Crypto-native builder audience |
| 3 | Bluesky | `@quincehq.bsky.social` | Cross-posted technical updates |
| 3 | LinkedIn | `0xItsss` personal profile + Quince company page | Partners, hiring, infrastructure audience |
| Later | Product Hunt | `Quince` | Launch only with a usable demo and onboarding |

Reserve the listed names now. Do not create a Telegram group or a subreddit
until there is an active audience. A quiet public chat looks abandoned.

## Profile copy

### Official X bio

```text
Rust-native runtime for systematic trading.
QFL · custom VM · risk-first execution.
Building in public by @0xitsss.
```

Website: `https://github.com/0xitsss/quince`

### Founder X bio

```text
Building Quince: Rust-native infrastructure for systematic trading.
VMs, compilers, risk systems, exchange plumbing.
```

### GitHub profile README opening

```text
I build Quince — a Rust-native runtime for systematic trading.
Strategies compile from QFL into a compact VM and execute behind explicit,
fail-closed risk controls.
```

Pin `quince` as the first repository. Enable GitHub Discussions. Add topics:
`rust`, `algorithmic-trading`, `quantitative-finance`, `trading`,
`virtual-machine`, `hyperliquid`, `binance`, `low-latency`.

## First official X post

Post this from the official Quince account and pin it:

```text
Quince is a Rust-native runtime for systematic trading.

It turns strategies written in QFL into bytecode executed by a custom VM, then
routes their intent through explicit risk controls and exchange adapters.

Why build it?

Most trading code starts as scripts and ends up carrying real money with:
• secrets in .env files
• risk checks detached from pending/open exposure
• adapters that pretend an order was placed
• "fast" paths nobody measures

Quince is built to make those failures explicit:
• OS-keychain wallet setup
• risk gates that fail closed
• public Binance + Hyperliquid market-data paths
• reproducible tests, CI, and benchmarks
• no live execution claim until signing and order flow are verified

v0.7.5 is out. The work is very much in progress.

https://github.com/0xitsss/quince
```

Attach the Quince hero image and a screenshot/GIF of a short QFL strategy next
to the compiled VM execution trace. Do not attach wallet screens, balances,
private endpoint URLs, or benchmark results without their methodology.

## First founder post

Post this from `@0xitsss` one day later:

```text
I am building Quince in public: a Rust-native runtime for systematic trading.

The interesting problem is not predicting a candle. It is turning strategy
intent into execution without lying about risk, credentials, latency, or what
the system can actually do.

The stack so far:
• QFL strategy language
• compiler + compact VM
• position, pending-order, daily-loss and drawdown controls
• Binance and Hyperliquid public market-data adapters
• OS-keychain wallet setup

If a path cannot safely execute yet, Quince refuses it. That is deliberate.

v0.7.5: https://github.com/0xitsss/quince
```

## Content cadence: first 30 days

Publish two or three useful posts each week. Every post should follow:

> problem → engineering decision → measurement/evidence → current limitation

| Week | Post | Desired outcome |
| --- | --- | --- |
| 1 | Launch post and founder post | Explain what Quince is and is not |
| 1 | "Why live execution fails closed" | Establish a safety standard |
| 2 | QFL source → bytecode → VM diagram | Explain the technical core |
| 2 | Benchmark post: measured regression, reverted change | Demonstrate engineering honesty |
| 3 | Risk controls: open position + pending exposure | Attract quant/execution feedback |
| 3 | Wallet-keychain design | Explain secret-handling model |
| 4 | Hyperliquid/Binance adapter status | Recruit testers without overstating support |
| 4 | Roadmap and one precise open question | Invite contributors and review |

### Post templates

**Build log**

```text
Build log: [change].

Problem: [concrete failure mode].
Decision: [what changed].
Evidence: [test/benchmark/review result].
Limit: [what remains unsupported].

Code: [link]
```

**Ask for technical feedback**

```text
I am reviewing Quince's [component] design.

Current invariant: [one clear invariant].
Open question: [one bounded technical question].

Relevant code + tests: [link]
```

## Community rules

### Reddit

Start by leaving useful comments in `r/rust`, `r/algotrading`, `r/quant`, and
`r/CryptoTechnology`. Read each community's self-promotion rules before posting.
Make one architecture-feedback post, not identical launch spam across every
subreddit. Do not ask for karma or advertise returns.

### Discord

Open only after repeated support requests. Initial channels:

```text
# announcements
# build-in-public
# qfl
# exchange-integrations
# help
# security
```

Pin this in `#security`:

> Never post a private key, seed phrase, API secret, wallet file, or balance.
> Quince contributors will never ask for one.

## Release routine

For every release:

1. Tag the version and create a GitHub Release.
2. Publish a three-bullet changelog on `@quincehq`.
3. Repost it from `@0xitsss` with one engineering detail or tradeoff.
4. Add a short discussion post if the release changes an API or safety model.
5. Reply to every substantive question within 48 hours.

## Safety and credibility checklist

- Enable passkeys or two-factor authentication on every account.
- Use a password manager and separate recovery codes.
- Never publish exchange API secrets, private keys, seed phrases, private logs,
  real balances, IP addresses, or unredacted error dumps.
- Label all market-data-only adapters as read-only.
- Publish benchmark methodology alongside performance claims.
- Write "research / paper-trading infrastructure" until verified live signing,
  order placement, reconciliation, and incident handling are complete.
