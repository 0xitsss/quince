use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter, ZScore,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_zscore",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[IndicatorParameter {
        name: "period",
        min: 2.,
        max: 100_000.,
    }],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Value(ZScore);
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p[0].fract() != 0. {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    Ok(Box::new(Value(ZScore::new(p[0] as usize))))
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
    fn params() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[1.]).is_err());
    }
    #[test]
    fn descriptor() {
        assert_eq!(DESCRIPTOR.name, "custom_zscore");
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
