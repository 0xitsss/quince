use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter, Macd,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_macd_signal",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[
        IndicatorParameter {
            name: "fast",
            min: 1.,
            max: 10_000.,
        },
        IndicatorParameter {
            name: "slow",
            min: 2.,
            max: 100_000.,
        },
        IndicatorParameter {
            name: "signal",
            min: 1.,
            max: 10_000.,
        },
    ],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Value(Macd);
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p.iter().any(|x| x.fract() != 0.) || p[0] >= p[1] {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "periods must be integral and fast less than slow",
        });
    }
    Ok(Box::new(Value(Macd::new(
        p[0] as usize,
        p[1] as usize,
        p[2] as usize,
    ))))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.0.update(t.price).map(|x| x.signal_line)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relation() {
        assert!(create(&[2., 4., 2.]).is_ok());
        assert!(create(&[4., 2., 2.]).is_err());
    }
    #[test]
    fn descriptor() {
        assert_eq!(DESCRIPTOR.parameters.len(), 3);
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
