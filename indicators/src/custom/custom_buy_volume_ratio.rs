// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Cumulative buy volume share.
use quince_core::types::{Side, Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_buy_volume_ratio",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    buy: f64,
    total: f64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator {
        buy: 0.0,
        total: 0.0,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.qty.is_finite() || t.qty <= 0.0 {
            return None;
        }
        self.total += t.qty;
        if t.side == Side::Buy {
            self.buy += t.qty;
        }
        let value = self.buy / self.total;
        value.is_finite().then_some(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(s: Side, q: f64) -> Trade {
        Trade {
            price: 1.0,
            qty: q,
            side: s,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn calculates_buy_share() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(Side::Buy, 3.0));
        assert_eq!(i.on_trade(&t(Side::Sell, 1.0)), Some(0.75));
    }
}
