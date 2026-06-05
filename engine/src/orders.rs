use quince_core::types::*;
use std::collections::HashMap;
use std::time::Instant;

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
    Waiting,
    Placed { order_id: String },
    PartiallyFilled { order_id: String, filled_qty: f64 },
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
        let client_id = format!("qc_{}", self.next_id);
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
            po.status = PendingStatus::Placed {
                order_id: order_id.clone(),
            };
            po.last_update = Instant::now();
            self.exchange_to_client.insert(order_id, client_id.to_string());
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
            po.status = PendingStatus::Filled;
            po.last_update = Instant::now();
        }
    }

    pub fn mark_failed(&mut self, client_id: &str, err: String) {
        if let Some(po) = self.orders.get_mut(client_id) {
            po.status = PendingStatus::Failed(err);
            po.last_update = Instant::now();
        }
    }

    pub fn cancel(&mut self, client_id: &str) {
        if let Some(po) = self.orders.get_mut(client_id) {
            po.status = PendingStatus::Cancelled;
            po.last_update = Instant::now();
        }
    }

    pub fn get(&self, client_id: &str) -> Option<&PendingOrder> {
        self.orders.get(client_id)
    }

    pub fn pending_order_ids(&self) -> Vec<String> {
        self.orders
            .iter()
            .filter_map(|(id, po)| match &po.status {
                PendingStatus::Waiting | PendingStatus::Placed { .. } | PendingStatus::PartiallyFilled { .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn cleanup_filled(&mut self) {
        self.orders.retain(|_, po| matches!(
            po.status,
            PendingStatus::Waiting | PendingStatus::Placed { .. } | PendingStatus::PartiallyFilled { .. }
        ));
    }

    /// Remove exchange->client mapping (call when order is fully done).
    pub fn remove_exchange_mapping(&mut self, exchange_id: &str) {
        self.exchange_to_client.remove(exchange_id);
    }

    pub fn find_client_by_exchange_id(&self, exchange_id: &str) -> Option<&str> {
        self.exchange_to_client.get(exchange_id).map(|s| s.as_str())
    }

    /// Update fill tracking. Returns true if order became fully filled.
    pub fn update_fill(&mut self, client_id: &str, qty: f64, price: f64) -> bool {
        if let Some(po) = self.orders.get_mut(client_id) {
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
                return true;
            }
            if let PendingStatus::Placed { order_id } = &po.status {
                po.status = PendingStatus::PartiallyFilled {
                    order_id: order_id.clone(),
                    filled_qty: po.filled_qty,
                };
            }
        }
        false
    }

    /// Returns all filled orders that have SL/TP levels active.
    pub fn active_sl_tp(&self) -> Vec<ActiveStop> {
        let has_any = self.orders.values().any(|po| {
            matches!(po.status, PendingStatus::Filled)
                && (po.order.stop_loss.is_some() || po.order.take_profit.is_some())
        });
        if !has_any { return Vec::new(); }

        self.orders
            .iter()
            .filter_map(|(id, po)| {
                if !matches!(po.status, PendingStatus::Filled) { return None; }
                let has_sl = po.order.stop_loss.is_some();
                let has_tp = po.order.take_profit.is_some();
                if !has_sl && !has_tp { return None; }
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
        assert_eq!(stops[0].side, Side::Sell); // buy→close with sell
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
        om.cancel(&id);
        let po = om.get(&id).unwrap();
        assert!(matches!(po.status, PendingStatus::Cancelled));
    }

    #[test]
    fn pending_order_ids_returns_only_active() {
        let mut om = OrderManager::new();
        let id1 = om.register(buy_order(None, None));
        let id2 = om.register(buy_order(None, None));
        om.mark_placed(&id1, "ex1".into());
        om.cancel(&id2);
        let pending = om.pending_order_ids();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], id1);
    }

    #[test]
    fn cleanup_filled_removes_non_active() {
        let mut om = OrderManager::new();
        let id1 = om.register(buy_order(None, None));
        let id2 = om.register(buy_order(None, None));
        om.mark_placed(&id1, "ex1".into());
        om.update_fill(&id1, 1.0, 100.0);
        om.cancel(&id2);
        om.cleanup_filled();
        assert_eq!(om.orders.len(), 0);
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
        assert_eq!(stops[0].side, Side::Buy); // sell→close with buy
    }
}
