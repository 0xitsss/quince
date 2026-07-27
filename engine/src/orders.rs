// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Order lifecycle management.
//! Tracks pending orders, active stop-loss/take-profit levels, and order fill
//! reconciliation via [`OrderManager`], [`PendingOrder`], and [`ActiveStop`].

use quince_core::types::*;
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ActiveStop {
    pub client_id: String,
    pub side: Side,
    pub qty: f64,
    pub entry_price: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum PendingStatus {
    /// The submission request has not produced a response yet.
    Waiting,
    Placed {
        order_id: String,
    },
    PartiallyFilled {
        order_id: String,
        filled_qty: f64,
    },
    /// A cancellation request was accepted by the transport, but has not yet
    /// been confirmed by the exchange's order state.
    CancelRequested {
        order_id: String,
    },
    /// The submission transport failed after the request may have reached the
    /// exchange.  Keep this order risk-visible; retrying blindly could double
    /// the position.
    SubmissionUnknown {
        error: String,
    },
    Filled,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct PendingOrder {
    pub client_id: String,
    pub order: Order,
    pub status: PendingStatus,
    pub placed_at: Instant,
    pub last_update: Instant,
    pub filled_qty: f64,
    pub avg_price: f64,
}

#[derive(Default)]
pub struct OrderManager {
    pub orders: HashMap<String, PendingOrder>,
    pub exchange_to_client: HashMap<String, String>,
    next_id: u64,
}

impl OrderManager {
    pub fn new() -> Self {
        Self {
            orders: HashMap::new(),
            exchange_to_client: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn register(&mut self, order: Order) -> String {
        // This is a local correlation ID, not an exchange idempotency key.  A
        // process-unique timestamp prevents collisions in logs/restarts while
        // the Exchange trait is unable to carry a client order ID upstream.
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let client_id = format!("qc_{epoch_nanos}_{:016x}", self.next_id);
        self.next_id += 1;
        let now = Instant::now();
        let po = PendingOrder {
            client_id: client_id.clone(),
            order,
            status: PendingStatus::Waiting,
            placed_at: now,
            last_update: now,
            filled_qty: 0.0,
            avg_price: 0.0,
        };
        self.orders.insert(client_id.clone(), po);
        client_id
    }

    pub fn mark_placed(&mut self, client_id: &str, order_id: String) {
        if let Some(po) = self.orders.get_mut(client_id) {
            if !matches!(po.status, PendingStatus::Waiting) {
                return;
            }
            po.status = PendingStatus::Placed {
                order_id: order_id.clone(),
            };
            po.last_update = Instant::now();
            self.exchange_to_client
                .insert(order_id, client_id.to_string());
        }
    }

    /// Preserve exposure when a placement result is ambiguous.  The caller
    /// must reconcile manually until the exchange API supports lookup by a
    /// client-generated idempotency key.
    pub fn mark_submission_unknown(&mut self, client_id: &str, error: String) {
        if let Some(po) = self.orders.get_mut(client_id) {
            if matches!(po.status, PendingStatus::Waiting) {
                po.status = PendingStatus::SubmissionUnknown { error };
                po.last_update = Instant::now();
            }
        }
    }

    pub fn mark_partial(&mut self, client_id: &str, order_id: &str, filled_qty: f64) {
        if let Some(po) = self.orders.get_mut(client_id) {
            po.status = PendingStatus::PartiallyFilled {
                order_id: order_id.to_string(),
                filled_qty,
            };
            po.last_update = Instant::now();
        }
    }

    pub fn mark_filled(&mut self, client_id: &str) {
        if let Some(po) = self.orders.get_mut(client_id) {
            if !Self::is_active_status(&po.status) {
                return;
            }
            po.status = PendingStatus::Filled;
            po.last_update = Instant::now();
            self.remove_client_exchange_mapping(client_id);
        }
    }

    pub fn mark_failed(&mut self, client_id: &str, err: String) {
        if let Some(po) = self.orders.get_mut(client_id) {
            if !matches!(po.status, PendingStatus::Waiting) {
                return;
            }
            po.status = PendingStatus::Failed(err);
            po.last_update = Instant::now();
            self.remove_client_exchange_mapping(client_id);
        }
    }

    /// Mark a terminal cancellation only after it is confirmed by the
    /// exchange (or a successful cancellation endpoint with that guarantee).
    pub fn mark_cancelled(&mut self, client_id: &str) {
        if let Some(po) = self.orders.get_mut(client_id) {
            if !Self::is_active_status(&po.status) {
                return;
            }
            po.status = PendingStatus::Cancelled;
            po.last_update = Instant::now();
            self.remove_client_exchange_mapping(client_id);
        }
    }

    /// Record a cancellation request without assuming that the order stopped
    /// matching.  The remaining quantity stays in risk exposure until a
    /// terminal order status is observed.
    pub fn mark_cancel_requested(&mut self, client_id: &str) {
        if let Some(po) = self.orders.get_mut(client_id) {
            let order_id = match &po.status {
                PendingStatus::Placed { order_id }
                | PendingStatus::PartiallyFilled { order_id, .. }
                | PendingStatus::CancelRequested { order_id } => order_id.clone(),
                _ => return,
            };
            po.status = PendingStatus::CancelRequested { order_id };
            po.last_update = Instant::now();
        }
    }

    pub fn get(&self, client_id: &str) -> Option<&PendingOrder> {
        self.orders.get(client_id)
    }

    pub fn has_pending(&self) -> bool {
        self.orders.values().any(|po| {
            matches!(
                po.status,
                PendingStatus::Waiting
                    | PendingStatus::Placed { .. }
                    | PendingStatus::PartiallyFilled { .. }
                    | PendingStatus::CancelRequested { .. }
                    | PendingStatus::SubmissionUnknown { .. }
            )
        })
    }

    pub fn pending_order_ids(&self) -> Vec<String> {
        self.orders
            .iter()
            .filter_map(|(id, po)| match &po.status {
                PendingStatus::Waiting
                | PendingStatus::Placed { .. }
                | PendingStatus::PartiallyFilled { .. }
                | PendingStatus::CancelRequested { .. }
                | PendingStatus::SubmissionUnknown { .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// Remove terminal orders that no longer carry an active protective stop.
    /// Filled entry orders with SL/TP must stay tracked until their protection
    /// is either triggered or explicitly deactivated.
    pub fn cleanup_terminal(&mut self) {
        let removed: Vec<String> = self
            .orders
            .iter()
            .filter_map(|(id, po)| {
                let is_pending = matches!(
                    po.status,
                    PendingStatus::Waiting
                        | PendingStatus::Placed { .. }
                        | PendingStatus::PartiallyFilled { .. }
                        | PendingStatus::CancelRequested { .. }
                        | PendingStatus::SubmissionUnknown { .. }
                );
                let has_active_protection =
                    matches!(po.status, PendingStatus::Filled | PendingStatus::Cancelled)
                        && po.filled_qty > 0.0
                        && (po.order.stop_loss.is_some() || po.order.take_profit.is_some());
                if !is_pending && !has_active_protection {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in &removed {
            self.remove_client_exchange_mapping(id);
        }
        self.orders.retain(|id, _| !removed.contains(id));
    }

    /// Net worst-case exposure from orders that may still fill.
    pub fn pending_signed_exposure(&self) -> f64 {
        self.orders
            .values()
            .filter(|po| {
                !po.order.reduce_only
                    && matches!(
                        po.status,
                        PendingStatus::Waiting
                            | PendingStatus::Placed { .. }
                            | PendingStatus::PartiallyFilled { .. }
                            | PendingStatus::CancelRequested { .. }
                            | PendingStatus::SubmissionUnknown { .. }
                    )
            })
            .map(|po| {
                let remaining = (po.order.qty - po.filled_qty).max(0.0);
                match po.order.side {
                    Side::Buy => remaining,
                    Side::Sell => -remaining,
                }
            })
            .sum()
    }

    /// Remove exchange->client mapping (call when order is fully done).
    pub fn remove_exchange_mapping(&mut self, exchange_id: &str) {
        self.exchange_to_client.remove(exchange_id);
    }

    /// Remove exchange mapping for a given client_id by scanning all entries.
    fn remove_client_exchange_mapping(&mut self, client_id: &str) {
        self.exchange_to_client.retain(|_, v| v != client_id);
    }

    pub fn find_client_by_exchange_id(&self, exchange_id: &str) -> Option<&str> {
        self.exchange_to_client.get(exchange_id).map(|s| s.as_str())
    }

    pub fn exchange_order_id(&self, client_id: &str) -> Option<&str> {
        match &self.orders.get(client_id)?.status {
            PendingStatus::Placed { order_id }
            | PendingStatus::PartiallyFilled { order_id, .. }
            | PendingStatus::CancelRequested { order_id } => Some(order_id),
            _ => None,
        }
    }

    /// Binds an order discovered through a native client-order-id lookup and
    /// reconciles its authoritative exchange state. This is the only safe way
    /// to recover a `SubmissionUnknown` order without issuing a duplicate.
    pub fn reconcile_client_order(
        &mut self,
        client_id: &str,
        exchange_order_id: &str,
        status: &str,
        filled_qty: f64,
        avg_price: f64,
    ) -> Result<(), String> {
        if exchange_order_id.trim().is_empty() {
            return Err("exchange returned an empty order ID for client reconciliation".into());
        }
        if self.exchange_order_id(client_id).is_none() {
            let po = self
                .orders
                .get_mut(client_id)
                .ok_or_else(|| "unknown client order".to_string())?;
            if !matches!(
                po.status,
                PendingStatus::Waiting | PendingStatus::SubmissionUnknown { .. }
            ) {
                return Err("client reconciliation attempted for an already-bound order".into());
            }
            po.status = PendingStatus::Placed {
                order_id: exchange_order_id.to_string(),
            };
            po.last_update = Instant::now();
            self.exchange_to_client
                .insert(exchange_order_id.to_string(), client_id.to_string());
        }
        self.reconcile_status(client_id, status, filled_qty, avg_price)
    }

    /// Reconcile lifecycle state from an authoritative exchange status.  This
    /// intentionally does not synthesize fills, PnL, or strategy callbacks:
    /// the compact `OrderStatus` type has no fill IDs, timestamps, or fees.
    pub fn reconcile_status(
        &mut self,
        client_id: &str,
        status: &str,
        filled_qty: f64,
        avg_price: f64,
    ) -> Result<(), String> {
        let normalized = status.trim().to_ascii_uppercase();
        let po = self
            .orders
            .get_mut(client_id)
            .ok_or_else(|| "unknown client order".to_string())?;
        if !Self::is_active_status(&po.status) {
            return Ok(());
        }
        if !filled_qty.is_finite() || filled_qty < 0.0 || filled_qty > po.order.qty + 1e-12 {
            return Err("exchange reported invalid cumulative fill quantity".into());
        }
        if filled_qty > po.filled_qty {
            po.filled_qty = filled_qty;
            if avg_price.is_finite() && avg_price > 0.0 {
                po.avg_price = avg_price;
            }
        }
        po.last_update = Instant::now();

        match normalized.as_str() {
            "NEW" | "OPEN" | "PENDING_NEW" => {}
            "PARTIALLY_FILLED" | "PARTIAL" => {
                let order_id = Self::order_id_from_status(&po.status)
                    .ok_or_else(|| "partial status has no exchange order id".to_string())?;
                po.status = PendingStatus::PartiallyFilled {
                    order_id,
                    filled_qty: po.filled_qty,
                };
            }
            "FILLED" => {
                po.filled_qty = po.order.qty;
                po.status = PendingStatus::Filled;
            }
            "CANCELED" | "CANCELLED" | "EXPIRED" => po.status = PendingStatus::Cancelled,
            "REJECTED" => po.status = PendingStatus::Failed("exchange rejected order".into()),
            _ => return Err(format!("unrecognized exchange order status: {status}")),
        }

        let terminal = matches!(
            po.status,
            PendingStatus::Filled | PendingStatus::Cancelled | PendingStatus::Failed(_)
        );
        if terminal {
            self.remove_client_exchange_mapping(client_id);
        }
        Ok(())
    }

    /// Update fill tracking. Returns true if order became fully filled.
    pub fn update_fill(&mut self, client_id: &str, qty: f64, price: f64) -> bool {
        if let Some(po) = self.orders.get_mut(client_id) {
            if !Self::is_active_status(&po.status)
                || !qty.is_finite()
                || qty <= 0.0
                || !price.is_finite()
                || price <= 0.0
            {
                return false;
            }
            let old_filled = po.filled_qty;
            po.filled_qty = (po.filled_qty + qty).min(po.order.qty);
            po.avg_price = if po.filled_qty > 0.0 {
                (old_filled * po.avg_price + qty * price) / po.filled_qty
            } else {
                price
            };
            po.last_update = Instant::now();

            if po.filled_qty >= po.order.qty - 1e-12 {
                po.status = PendingStatus::Filled;
                self.remove_client_exchange_mapping(client_id);
                return true;
            }
            if let Some(order_id) = Self::order_id_from_status(&po.status) {
                po.status = PendingStatus::PartiallyFilled {
                    order_id,
                    filled_qty: po.filled_qty,
                };
            }
        }
        false
    }

    /// Returns all filled orders that have SL/TP levels active.
    pub fn active_sl_tp(&self) -> Vec<ActiveStop> {
        let has_any = self.orders.values().any(|po| {
            matches!(po.status, PendingStatus::Filled | PendingStatus::Cancelled)
                && po.filled_qty > 0.0
                && (po.order.stop_loss.is_some() || po.order.take_profit.is_some())
        });
        if !has_any {
            return Vec::new();
        }

        self.orders
            .iter()
            .filter_map(|(id, po)| {
                if !matches!(po.status, PendingStatus::Filled | PendingStatus::Cancelled)
                    || po.filled_qty <= 0.0
                {
                    return None;
                }
                let has_sl = po.order.stop_loss.is_some();
                let has_tp = po.order.take_profit.is_some();
                if !has_sl && !has_tp {
                    return None;
                }
                let close_side = match po.order.side {
                    Side::Buy => Side::Sell,
                    Side::Sell => Side::Buy,
                };
                Some(ActiveStop {
                    client_id: id.clone(),
                    side: close_side,
                    qty: po.filled_qty,
                    entry_price: po.avg_price,
                    stop_loss: po.order.stop_loss,
                    take_profit: po.order.take_profit,
                })
            })
            .collect()
    }

    /// Remove SL/TP tracking after it's triggered.
    pub fn deactivate_sl_tp(&mut self, client_id: &str) {
        if let Some(po) = self.orders.get_mut(client_id) {
            po.order.stop_loss = None;
            po.order.take_profit = None;
        }
    }

    fn is_active_status(status: &PendingStatus) -> bool {
        matches!(
            status,
            PendingStatus::Waiting
                | PendingStatus::Placed { .. }
                | PendingStatus::PartiallyFilled { .. }
                | PendingStatus::CancelRequested { .. }
                | PendingStatus::SubmissionUnknown { .. }
        )
    }

    fn order_id_from_status(status: &PendingStatus) -> Option<String> {
        match status {
            PendingStatus::Placed { order_id }
            | PendingStatus::PartiallyFilled { order_id, .. }
            | PendingStatus::CancelRequested { order_id } => Some(order_id.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buy_order(sl: Option<f64>, tp: Option<f64>) -> Order {
        Order {
            symbol: "btcusdt".into(),
            side: Side::Buy,
            qty: 1.0,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: sl,
            take_profit: tp,
        }
    }

    #[test]
    fn register_order_with_sl_tp() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(Some(99.0), Some(101.0)));
        let po = om.get(&id).unwrap();
        assert_eq!(po.order.stop_loss, Some(99.0));
        assert_eq!(po.order.take_profit, Some(101.0));
        assert_eq!(po.filled_qty, 0.0);
        assert_eq!(po.avg_price, 0.0);
    }

    #[test]
    fn update_fill_partial() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        assert!(!om.update_fill(&id, 0.3, 100.0));
        let po = om.get(&id).unwrap();
        assert!((po.filled_qty - 0.3).abs() < 1e-12);
        assert!((po.avg_price - 100.0).abs() < 1e-12);
        assert!(matches!(po.status, PendingStatus::PartiallyFilled { .. }));
    }

    #[test]
    fn update_fill_full() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        assert!(om.update_fill(&id, 1.0, 100.0));
        let po = om.get(&id).unwrap();
        assert!((po.filled_qty - 1.0).abs() < 1e-12);
        assert!(matches!(po.status, PendingStatus::Filled));
    }

    #[test]
    fn active_sl_tp_returns_only_filled_orders_with_levels() {
        let mut om = OrderManager::new();
        let id1 = om.register(buy_order(Some(99.0), Some(101.0))); // buy, has sl/tp
        let id2 = om.register(buy_order(None, None)); // buy, no sl/tp
        om.mark_placed(&id1, "ex1".into());
        om.mark_placed(&id2, "ex2".into());

        // Not filled yet - should return empty
        assert!(om.active_sl_tp().is_empty());

        // Fill id1
        om.update_fill(&id1, 1.0, 100.0);
        let stops = om.active_sl_tp();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].client_id, id1);
        assert_eq!(stops[0].side, Side::Sell); // buyв†’close with sell
        assert_eq!(stops[0].stop_loss, Some(99.0));
        assert_eq!(stops[0].take_profit, Some(101.0));

        // Fill id2 - should NOT be in active stops (no sl/tp)
        om.update_fill(&id2, 1.0, 100.0);
        assert_eq!(om.active_sl_tp().len(), 1);
    }

    #[test]
    fn deactivate_sl_tp_clears_levels() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(Some(99.0), None));
        om.mark_placed(&id, "ex1".into());
        om.update_fill(&id, 1.0, 100.0);
        assert_eq!(om.active_sl_tp().len(), 1);

        om.deactivate_sl_tp(&id);
        assert!(om.active_sl_tp().is_empty());
    }

    #[test]
    fn update_fill_weighted_avg_price() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        om.update_fill(&id, 0.5, 100.0);
        om.update_fill(&id, 0.5, 102.0);
        let po = om.get(&id).unwrap();
        assert!((po.avg_price - 101.0).abs() < 1e-12); // (0.5*100 + 0.5*102) / 1.0
        assert!(matches!(po.status, PendingStatus::Filled));
    }

    #[test]
    fn new_order_manager_empty() {
        let om = OrderManager::new();
        assert!(om.orders.is_empty());
        assert!(om.exchange_to_client.is_empty());
    }

    #[test]
    fn register_creates_waiting_order() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        let po = om.get(&id).unwrap();
        assert!(matches!(po.status, PendingStatus::Waiting));
    }

    #[test]
    fn mark_placed_updates_status() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "exchange_1".into());
        let po = om.get(&id).unwrap();
        assert!(matches!(po.status, PendingStatus::Placed { .. }));
    }

    #[test]
    fn mark_failed_updates_status() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_failed(&id, "insufficient funds".into());
        let po = om.get(&id).unwrap();
        assert!(matches!(po.status, PendingStatus::Failed(_)));
    }

    #[test]
    fn mark_cancelled_updates_status() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_cancelled(&id);
        let po = om.get(&id).unwrap();
        assert!(matches!(po.status, PendingStatus::Cancelled));
    }

    #[test]
    fn pending_order_ids_returns_only_active() {
        let mut om = OrderManager::new();
        let id1 = om.register(buy_order(None, None));
        let id2 = om.register(buy_order(None, None));
        om.mark_placed(&id1, "ex1".into());
        om.mark_cancelled(&id2);
        let pending = om.pending_order_ids();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], id1);
    }

    #[test]
    fn cleanup_terminal_preserves_filled_order_with_sl_tp() {
        let mut om = OrderManager::new();
        let id1 = om.register(buy_order(Some(99.0), None));
        let id2 = om.register(buy_order(None, None));
        om.mark_placed(&id1, "ex1".into());
        om.update_fill(&id1, 1.0, 100.0);
        om.mark_cancelled(&id2);
        om.cleanup_terminal();
        assert_eq!(om.orders.len(), 1);
        assert!(om.get(&id1).is_some());
    }

    #[test]
    fn pending_signed_exposure_uses_unfilled_quantity() {
        let mut om = OrderManager::new();
        let buy = om.register(buy_order(None, None));
        let mut sell_order = buy_order(None, None);
        sell_order.side = Side::Sell;
        let sell = om.register(sell_order);
        om.mark_placed(&buy, "buy".into());
        om.mark_placed(&sell, "sell".into());
        om.update_fill(&buy, 0.25, 100.0);

        assert!((om.pending_signed_exposure() + 0.25).abs() < 1e-12);
    }

    #[test]
    fn find_client_by_exchange_id() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        assert_eq!(om.find_client_by_exchange_id("ex_id"), Some(id.as_str()));
        assert_eq!(om.find_client_by_exchange_id("unknown"), None);
    }

    #[test]
    fn remove_exchange_mapping() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        om.remove_exchange_mapping("ex_id");
        assert_eq!(om.find_client_by_exchange_id("ex_id"), None);
    }

    #[test]
    fn active_sl_tp_for_sell_order_returns_buy() {
        let mut om = OrderManager::new();
        let order = Order {
            symbol: "btcusdt".into(),
            side: Side::Sell,
            qty: 1.0,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: Some(110.0),
            take_profit: Some(90.0),
        };
        let id = om.register(order);
        om.mark_placed(&id, "ex1".into());
        om.update_fill(&id, 1.0, 100.0);
        let stops = om.active_sl_tp();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].side, Side::Buy); // sellв†’close with buy
    }

    #[test]
    fn ambiguous_submission_remains_risk_visible() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_submission_unknown(&id, "connection dropped".into());

        assert!(matches!(
            om.get(&id).unwrap().status,
            PendingStatus::SubmissionUnknown { .. }
        ));
        assert_eq!(om.pending_order_ids(), vec![id]);
        assert!((om.pending_signed_exposure() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cancellation_request_does_not_drop_exposure() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        om.mark_cancel_requested(&id);

        assert!(matches!(
            om.get(&id).unwrap().status,
            PendingStatus::CancelRequested { .. }
        ));
        assert_eq!(om.exchange_order_id(&id), Some("ex_id"));
        assert!((om.pending_signed_exposure() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn client_id_reconciliation_binds_an_ambiguous_submission_without_retrying() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_submission_unknown(&id, "connection dropped".into());

        om.reconcile_client_order(&id, "exchange-42", "NEW", 0.0, 0.0)
            .unwrap();

        assert_eq!(om.exchange_order_id(&id), Some("exchange-42"));
        assert_eq!(
            om.find_client_by_exchange_id("exchange-42"),
            Some(id.as_str())
        );
        assert!((om.pending_signed_exposure() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn reconciliation_only_removes_exposure_after_terminal_status() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        om.mark_cancel_requested(&id);
        om.reconcile_status(&id, "CANCELED", 0.0, 0.0).unwrap();

        assert!(matches!(
            om.get(&id).unwrap().status,
            PendingStatus::Cancelled
        ));
        assert_eq!(om.find_client_by_exchange_id("ex_id"), None);
        assert_eq!(om.pending_signed_exposure(), 0.0);
    }

    #[test]
    fn partial_fill_cancel_keeps_protective_stop_active() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(Some(99.0), None));
        om.mark_placed(&id, "ex_id".into());
        om.reconcile_status(&id, "CANCELED", 0.4, 100.0).unwrap();

        let stops = om.active_sl_tp();
        assert_eq!(stops.len(), 1);
        assert!((stops[0].qty - 0.4).abs() < 1e-12);
        om.cleanup_terminal();
        assert!(om.get(&id).is_some());
    }

    #[test]
    fn terminal_order_cannot_be_resurrected_by_late_updates() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());
        om.mark_cancelled(&id);

        assert!(!om.update_fill(&id, 1.0, 100.0));
        om.mark_placed(&id, "different_id".into());
        assert!(matches!(
            om.get(&id).unwrap().status,
            PendingStatus::Cancelled
        ));
    }

    #[test]
    fn reconciliation_rejects_impossible_cumulative_fill() {
        let mut om = OrderManager::new();
        let id = om.register(buy_order(None, None));
        om.mark_placed(&id, "ex_id".into());

        assert!(om.reconcile_status(&id, "FILLED", 2.0, 100.0).is_err());
        assert!(matches!(
            om.get(&id).unwrap().status,
            PendingStatus::Placed { .. }
        ));
    }
}
