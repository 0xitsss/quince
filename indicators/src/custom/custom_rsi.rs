use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter, Rsi,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_rsi",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[IndicatorParameter {
        name: "period",
        min: 1.,
        max: 100_000.,
    }],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Value(Rsi);
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p[0].fract() != 0. {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    Ok(Box::new(Value(Rsi::new(p[0] as usize))))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.0.update(t.price)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn t(p: f64) -> Trade {
        Trade {
            price: p,
            qty: 1.,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        }
    }
    #[test]
    fn invalid() {
        assert!(create(&[0.]).is_err());
        assert!(create(&[2.]).is_ok());
    }
    #[test]
    fn gain() {
        let mut x = create(&[2.]).unwrap();
        x.on_trade(&t(1.));
        x.on_trade(&t(2.));
        assert_eq!(x.on_trade(&t(3.)), Some(100.));
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
