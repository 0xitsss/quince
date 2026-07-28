use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, Ema, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_ema",
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
struct Value(Ema);
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p[0].fract() != 0. {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    Ok(Box::new(Value(Ema::new(p[0] as usize))))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        Some(self.0.update(t.price))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract() {
        assert!(create(&[3.0]).is_ok());
        assert!(create(&[1.5]).is_err());
        assert!(create(&[f64::NAN]).is_err());
    }
    #[test]
    fn value() {
        let mut x = create(&[3.0]).unwrap();
        assert_eq!(x.on_trade(&trade(2.0)), Some(2.0));
        assert!(x.on_trade(&trade(4.0)).unwrap() > 2.0);
    }
    fn trade(price: f64) -> Trade {
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
