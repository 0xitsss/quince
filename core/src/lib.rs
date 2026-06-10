// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Core types and data structures shared across all Quince crates.
//!
//! Provides [`RingVec`](ring::RingVec), [`RingBuffer`](ring::RingBuffer),
//! and domain types (`Trade`, `Depth`, `Order`, `OrderFill`, `Side`, etc.)
//! from [`types`].

pub mod ring;
pub mod types;
