// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL event tracer — ring buffer for strategy execution events.
//!
//! Records [`TraceEvent`]s (Signal, Feature, Fill, RiskAction) for post-hoc
//! analysis. Fixed-capacity ring buffer; drops oldest events when full.
//!
//! Entry point: [`Tracer::record()`].

use crate::opcodes::Opcode;

/// A recorded event for post-hoc analysis of strategy execution.
///
/// Each variant carries domain-specific payload:
/// - `Signal`: a strategy signal (opcode comparison result)
/// - `Feature`: a computed feature value (e.g. EMA, SMA)
/// - `Fill`: an executed order fill
/// - `RiskAction`: a risk engine verdict
#[derive(Debug, Clone)]
pub enum TraceEvent {
    Signal { kind: String, result: bool },
    Feature { name: String, value: f64 },
    Fill { price: f64, qty: f64, side: String },
    RiskAction { verdict: String, reason: String },
}

impl TraceEvent {
    /// Returns the event category as a static string: "signal", "feature", "fill", or "risk".
    pub fn kind(&self) -> &str {
        match self {
            TraceEvent::Signal { .. } => "signal",
            TraceEvent::Feature { .. } => "feature",
            TraceEvent::Fill { .. } => "fill",
            TraceEvent::RiskAction { .. } => "risk",
        }
    }
}

/// Ring-buffer event tracer for strategy execution.
///
/// Records signals, features, fills, and risk actions for post-hoc analysis.
/// Zero-allocation in the hot path when capacity is 0.
#[derive(Debug, Clone)]
pub struct Tracer {
    events: Vec<TraceEvent>,
    capacity: usize,
}

impl Tracer {
    /// Create a new tracer with the given ring-buffer capacity.
    /// A capacity of 0 disables all recording (zero-allocation).
    pub fn new(capacity: usize) -> Self {
        Tracer {
            events: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Record an event into the ring buffer. Drops oldest if at capacity.
    /// No-op when capacity is 0.
    pub fn record(&mut self, event: TraceEvent) {
        if self.capacity == 0 {
            return;
        }
        if self.events.len() >= self.capacity {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    /// Take all recorded events, clearing the buffer.
    pub fn drain(&mut self) -> Vec<TraceEvent> {
        std::mem::take(&mut self.events)
    }

    /// Number of events currently in the buffer.
    pub fn len(&self) -> usize {
        self.events.len()
    }
    /// True when no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    /// Remove all events from the buffer.
    pub fn clear(&mut self) {
        self.events.clear();
    }
    /// Return a slice of all buffered events.
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
    /// Maximum number of events the buffer can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Record a strategy signal event from an opcode comparison.
    pub fn record_signal(&mut self, op: Opcode, result: bool) {
        self.record(TraceEvent::Signal {
            kind: format!("{:?}", op),
            result,
        });
    }

    /// Record a feature computation event (e.g. an indicator value).
    pub fn record_feature(&mut self, name: &str, value: f64) {
        self.record(TraceEvent::Feature {
            name: name.to_string(),
            value,
        });
    }

    /// Record an order fill event.
    pub fn record_fill(&mut self, price: f64, qty: f64, side: &str) {
        self.record(TraceEvent::Fill {
            price,
            qty,
            side: side.to_string(),
        });
    }

    /// Record a risk engine verdict event.
    pub fn record_risk(&mut self, verdict: &str, reason: &str) {
        self.record(TraceEvent::RiskAction {
            verdict: verdict.to_string(),
            reason: reason.to_string(),
        });
    }
}

/// Default tracer with a capacity of 1024 events.
impl Default for Tracer {
    fn default() -> Self {
        Tracer::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::Opcode as O;

    #[test]
    fn new_tracer_has_no_events() {
        let t = Tracer::new(1024);
        assert_eq!(t.len(), 0);
        assert!(t.is_empty());
    }

    #[test]
    fn record_signal_event() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::Signal {
            kind: "Gt".into(),
            result: true,
        });
        assert_eq!(t.len(), 1);
        assert_eq!(t.events()[0].kind(), "signal");
    }

    #[test]
    fn record_feature_event() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::Feature {
            name: "ema".into(),
            value: 50000.0,
        });
        assert_eq!(t.len(), 1);
        assert_eq!(t.events()[0].kind(), "feature");
    }

    #[test]
    fn record_fill_event() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::Fill {
            price: 50000.0,
            qty: 0.1,
            side: "Buy".into(),
        });
        assert_eq!(t.len(), 1);
        assert_eq!(t.events()[0].kind(), "fill");
    }

    #[test]
    fn record_risk_event() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::RiskAction {
            verdict: "allowed".into(),
            reason: "".into(),
        });
        assert_eq!(t.len(), 1);
        assert_eq!(t.events()[0].kind(), "risk");
    }

    #[test]
    fn drain_returns_all_events_and_clears() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::Signal {
            kind: "Gt".into(),
            result: true,
        });
        t.record(TraceEvent::Feature {
            name: "ema".into(),
            value: 1.0,
        });
        assert_eq!(t.drain().len(), 2);
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn tracer_respects_capacity_drops_oldest() {
        let mut t = Tracer::new(3);
        t.record(TraceEvent::Signal {
            kind: "a".into(),
            result: true,
        });
        t.record(TraceEvent::Signal {
            kind: "b".into(),
            result: false,
        });
        t.record(TraceEvent::Signal {
            kind: "c".into(),
            result: true,
        });
        t.record(TraceEvent::Signal {
            kind: "d".into(),
            result: false,
        });
        assert_eq!(t.len(), 3);
        let events = t.drain();
        assert_eq!(events[0].kind(), "signal");
        assert_eq!(events[1].kind(), "signal");
        assert_eq!(events[2].kind(), "signal");
    }

    #[test]
    fn clear_empties_tracer() {
        let mut t = Tracer::new(1024);
        t.record(TraceEvent::Signal {
            kind: "Gt".into(),
            result: true,
        });
        t.clear();
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn default_tracer_has_capacity_1024() {
        let t = Tracer::default();
        assert_eq!(t.capacity(), 1024);
    }

    #[test]
    fn record_signal_helper() {
        let mut t = Tracer::new(1024);
        t.record_signal(O::Gt, true);
        assert_eq!(t.len(), 1);
        match &t.events()[0] {
            TraceEvent::Signal { kind, result } => {
                assert_eq!(kind, "Gt");
                assert!(result);
            }
            _ => panic!("expected signal"),
        }
    }

    #[test]
    fn record_feature_helper() {
        let mut t = Tracer::new(1024);
        t.record_feature("ema", 1.5);
        assert_eq!(t.len(), 1);
        match &t.events()[0] {
            TraceEvent::Feature { name, value } => {
                assert_eq!(name, "ema");
                assert!((*value - 1.5).abs() < 0.001);
            }
            _ => panic!("expected feature"),
        }
    }

    #[test]
    fn record_fill_helper() {
        let mut t = Tracer::new(1024);
        t.record_fill(50000.0, 0.1, "Buy");
        assert_eq!(t.len(), 1);
        match &t.events()[0] {
            TraceEvent::Fill { price, qty, side } => {
                assert!((*price - 50000.0).abs() < 0.001);
                assert!((*qty - 0.1).abs() < 0.001);
                assert_eq!(side, "Buy");
            }
            _ => panic!("expected fill"),
        }
    }

    #[test]
    fn record_risk_helper() {
        let mut t = Tracer::new(1024);
        t.record_risk("rejected", "max position exceeded");
        assert_eq!(t.len(), 1);
        match &t.events()[0] {
            TraceEvent::RiskAction { verdict, reason } => {
                assert_eq!(verdict, "rejected");
                assert_eq!(reason, "max position exceeded");
            }
            _ => panic!("expected risk"),
        }
    }

    #[test]
    fn multiple_event_types_mixed() {
        let mut t = Tracer::new(1024);
        t.record_signal(O::Gt, true);
        t.record_feature("ema", 1.0);
        t.record_fill(100.0, 0.5, "Sell");
        t.record_risk("allowed", "");
        assert_eq!(t.len(), 4);
        assert_eq!(t.events()[0].kind(), "signal");
        assert_eq!(t.events()[1].kind(), "feature");
        assert_eq!(t.events()[2].kind(), "fill");
        assert_eq!(t.events()[3].kind(), "risk");
    }

    #[test]
    fn drain_on_empty_tracer_returns_empty_vec() {
        let mut t = Tracer::new(1024);
        let drained = t.drain();
        assert!(drained.is_empty());
    }

    #[test]
    fn capacity_zero_never_stores_events() {
        let mut t = Tracer::new(0);
        t.record(TraceEvent::Signal {
            kind: "Gt".into(),
            result: true,
        });
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn events_returns_slice_of_live_events() {
        let mut t = Tracer::new(1024);
        t.record_signal(O::Lt, false);
        let slice = t.events();
        assert_eq!(slice.len(), 1);
    }
}
