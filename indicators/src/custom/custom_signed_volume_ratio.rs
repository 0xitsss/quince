// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Cumulative signed volume divided by cumulative volume.
use quince_core::types::{Side, Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_signed_volume_ratio",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    signed: f64,
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
        signed: 0.0,
        total: 0.0,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.qty.is_finite() || t.qty <= 0.0 {
            return None;
        }
        self.total += t.qty;
        self.signed += if t.side == Side::Buy { t.qty } else { -t.qty };
        let value = self.signed / self.total;
        value.is_finite().then_some(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(side: Side, qty: f64) -> Trade {
        Trade {
            price: 1.0,
            qty,
            side,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn ratio_is_bounded_and_signed() {
        let mut i = create(&[]).unwrap();
        assert_eq!(i.on_trade(&t(Side::Buy, 3.0)), Some(1.0));
        assert_eq!(i.on_trade(&t(Side::Sell, 1.0)), Some(0.5));
    }
}
