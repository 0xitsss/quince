use quince_core::{ring::RingVec, types::Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_momentum",
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
struct Value {
    b: RingVec,
    p: usize,
}
fn create(x: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(x)?;
    if x[0].fract() != 0. {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    Ok(Box::new(Value {
        b: RingVec::new(x[0] as usize + 1),
        p: x[0] as usize,
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.b.push(t.price);
        (self.b.len() == self.p + 1).then(|| t.price - self.b.get(0).unwrap())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[2.5]).is_err());
    }
    #[test]
    fn warmup() {
        let mut x = create(&[1.]).unwrap();
        assert!(x.on_trade(&t(1.)).is_none());
        assert_eq!(x.on_trade(&t(3.)), Some(2.));
    }
    fn t(price: f64) -> Trade {
        Trade {
            price,
            qty: 1.,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        }
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
