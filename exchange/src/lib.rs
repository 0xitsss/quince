// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Exchange abstraction layer.
//! Defines the [`Exchange`](r#trait::Exchange) trait and provides exchange-specific
//! implementations (Binance REST + WebSocket).

pub mod binance;
pub mod r#trait;
