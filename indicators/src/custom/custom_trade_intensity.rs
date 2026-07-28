// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Trades per elapsed second since the first valid trade.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_trade_intensity",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    first: Option<i64>,
    count: u64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator {
        first: None,
        count: 0,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        let now = t.time.timestamp_millis();
        let first = *self.first.get_or_insert(now);
        self.count = self.count.saturating_add(1);
        let elapsed = (now - first) as f64 / 1000.0;
        if elapsed <= 0.0 {
            None
        } else {
            let v = self.count as f64 / elapsed;
            v.is_finite().then_some(v)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    fn t(ms: i64) -> Trade {
        Trade {
            price: 1.,
            qty: 1.,
            side: quince_core::types::Side::Buy,
            time: Utc.timestamp_millis_opt(ms).unwrap(),
            trade_id: 1,
        }
    }
    #[test]
    fn uses_elapsed_time() {
        let mut i = create(&[]).unwrap();
        assert_eq!(i.on_trade(&t(0)), None);
        assert_eq!(i.on_trade(&t(1000)), Some(2.0));
    }
}
