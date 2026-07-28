// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Running midpoint of the observed trade-price range.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_median_price",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    low: f64,
    high: f64,
    seen: bool,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator {
        low: 0.,
        high: 0.,
        seen: false,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() {
            return None;
        }
        if !self.seen {
            self.low = t.price;
            self.high = t.price;
            self.seen = true;
        } else {
            self.low = self.low.min(t.price);
            self.high = self.high.max(t.price);
        }
        let v = (self.low + self.high) * 0.5;
        v.is_finite().then_some(v)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(p: f64) -> Trade {
        Trade {
            price: p,
            qty: 1.,
            side: quince_core::types::Side::Buy,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn tracks_range_midpoint() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(10.));
        i.on_trade(&t(4.));
        assert_eq!(i.on_trade(&t(20.)), Some(12.));
    }
}
