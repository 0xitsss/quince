// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Latest non-zero price tick direction: -1, 0, or 1.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_tick_direction",
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
        if !t.price.is_finite() {
            return None;
        }
        self.previous
            .replace(t.price)
            .map(|previous| (t.price - previous).signum())
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
    fn returns_direction() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(2.));
        assert_eq!(i.on_trade(&t(1.)), Some(-1.));
    }
}
