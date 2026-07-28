// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Net count of buyer-initiated minus seller-initiated trades.
use quince_core::types::{Side, Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_trade_imbalance",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    value: f64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator { value: 0.0 }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.value += if t.side == Side::Buy { 1.0 } else { -1.0 };
        Some(self.value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(s: Side) -> Trade {
        Trade {
            price: 1.,
            qty: 1.,
            side: s,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn counts_sides() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(Side::Buy));
        assert_eq!(i.on_trade(&t(Side::Sell)), Some(0.));
    }
}
