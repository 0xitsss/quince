use quince_core::types::*;
use std::collections::HashMap;
use std::time::Instant;

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
}

pub struct OrderManager {
    pub orders: HashMap<String, PendingOrder>,
    next_id: u64,
}

impl OrderManager {
    pub fn new() -> Self {
        Self {
            orders: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn register(&mut self, order: Order) -> String {
        let client_id = format!("qc_{}", self.next_id);
        self.next_id += 1;
        self.orders.insert(
            client_id.clone(),
            PendingOrder {
                client_id: client_id.clone(),
                order,
                status: PendingStatus::Waiting,
                placed_at: Instant::now(),
                last_update: Instant::now(),
            },
        );
        client_id
    }

    pub fn mark_placed(&mut self, client_id: &str, order_id: String) {
        if let Some(po) = self.orders.get_mut(client_id) {
            po.status = PendingStatus::Placed {
                order_id: order_id.clone(),
            };
            po.last_update = Instant::now();
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
}
