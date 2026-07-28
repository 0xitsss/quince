use quince_core::{ring::RingVec, types::Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_kama",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[
        IndicatorParameter {
            name: "period",
            min: 2.,
            max: 100_000.,
        },
        IndicatorParameter {
            name: "fast",
            min: 1.,
            max: 1000.,
        },
        IndicatorParameter {
            name: "slow",
            min: 2.,
            max: 1000.,
        },
    ],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Value {
    b: RingVec,
    k: Option<f64>,
    fast: f64,
    slow: f64,
    p: usize,
}
fn create(x: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(x)?;
    if x[0].fract() != 0. || x[1].fract() != 0. || x[2].fract() != 0. || x[1] >= x[2] {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "periods must be integral and fast less than slow",
        });
    }
    Ok(Box::new(Value {
        b: RingVec::new(x[0] as usize + 1),
        k: None,
        fast: 2. / (x[1] + 1.),
        slow: 2. / (x[2] + 1.),
        p: x[0] as usize,
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        self.b.push(t.price);
        let prev = self.k.unwrap_or(t.price);
        if self.b.len() < self.p + 1 {
            self.k = Some(t.price);
            return None;
        }
        let change = (t.price - self.b.get(0).unwrap()).abs();
        let volatility = self
            .b
            .iter()
            .zip(self.b.iter().skip(1))
            .map(|(a, b)| (b - a).abs())
            .sum::<f64>();
        let er = if volatility == 0. {
            0.
        } else {
            change / volatility
        };
        let sc = (er * (self.fast - self.slow) + self.slow).powi(2);
        let v = prev + sc * (t.price - prev);
        self.k = Some(v);
        Some(v)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[3., 2., 10.]).is_ok());
        assert!(create(&[3., 10., 2.]).is_err());
    }
    #[test]
    fn warmup() {
        let mut x = create(&[2., 2., 10.]).unwrap();
        assert!(x.on_trade(&t(1.)).is_none());
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
