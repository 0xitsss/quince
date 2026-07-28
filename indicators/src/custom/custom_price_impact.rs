// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Absolute price change per unit of the current trade quantity.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_price_impact",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    previous: Option<f64>,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator { previous: None }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() || !t.qty.is_finite() || t.qty <= 0. {
            return None;
        }
        let previous = self.previous.replace(t.price)?;
        let v = (t.price - previous).abs() / t.qty;
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
            qty: 2.,
            side: quince_core::types::Side::Buy,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn measures_move_per_qty() {
        let mut i = create(&[]).unwrap();
        assert_eq!(i.on_trade(&t(10.)), None);
        assert_eq!(i.on_trade(&t(14.)), Some(2.));
    }
}
