# Module: `lib`

> Source: `lib.rs`

Quince trading engine — event loop, order manager, indicator bank.

The [`Engine`](loop::Engine) drives the strategy lifecycle: feeds market
data into the QFL runtime, dispatches orders, manages hot-reload, and
coordinates with the exchange connector.
