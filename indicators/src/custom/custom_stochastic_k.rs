use quince_core::{ring::RingVec, types::Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_stochastic_k",
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
    let p = x[0] as usize;
    Ok(Box::new(Value {
        b: RingVec::new(p),
        p,
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.b.push(t.price);
        if self.b.len() != self.p {
            return None;
        }
        let hi = self.b.iter().fold(f64::NEG_INFINITY, f64::max);
        let lo = self.b.iter().fold(f64::INFINITY, f64::min);
        if hi == lo {
            Some(50.)
        } else {
            Some(100. * (t.price - lo) / (hi - lo))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[2.2]).is_err());
    }
    #[test]
    fn range() {
        let mut x = create(&[2.]).unwrap();
        x.on_trade(&t(1.));
        assert_eq!(x.on_trade(&t(2.)), Some(100.));
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
