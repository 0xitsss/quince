use quince_core::{ring::RingVec, types::Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_linear_regression",
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
        let n = self.p as f64;
        let sx = n * (n - 1.) / 2.;
        let sx2 = (n - 1.) * n * (2. * n - 1.) / 6.;
        let sy: f64 = self.b.iter().sum();
        let sxy: f64 = self.b.iter().enumerate().map(|(i, y)| i as f64 * y).sum();
        let slope = (n * sxy - sx * sy) / (n * sx2 - sx * sx);
        Some((sy - slope * sx) / n + slope * (n - 1.))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[1.]).is_err());
    }
    #[test]
    fn line() {
        let mut x = create(&[3.]).unwrap();
        x.on_trade(&t(1.));
        x.on_trade(&t(2.));
        assert_eq!(x.on_trade(&t(3.)), Some(3.));
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
