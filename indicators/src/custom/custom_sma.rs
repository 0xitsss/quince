use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter, Sma,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_sma",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[IndicatorParameter {
        name: "period",
        min: 1.0,
        max: 100_000.0,
    }],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Value(Sma);
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p[0].fract() != 0.0 {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    Ok(Box::new(Value(Sma::new(p[0] as usize))))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.0.update(t.price)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract() {
        assert!(create(&[3.0]).is_ok());
        assert!(create(&[0.0]).is_err());
    }
    #[test]
    fn warmup() {
        let mut x = create(&[2.0]).unwrap();
        assert!(x.on_trade(&test_trade(1.0)).is_none());
        assert_eq!(x.on_trade(&test_trade(3.0)), Some(2.0));
    }
    fn test_trade(price: f64) -> Trade {
        Trade {
            price,
            qty: 1.0,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        }
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
