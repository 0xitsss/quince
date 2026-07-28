use quince_core::{ring::RingVec, types::Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_vortex",
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
    p: usize,
    prices: RingVec,
    vm: RingVec,
    tr: RingVec,
    prev: Option<f64>,
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
        p,
        prices: RingVec::new(p),
        vm: RingVec::new(p - 1),
        tr: RingVec::new(p - 1),
        prev: None,
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if let Some(prev) = self.prev {
            self.vm.push((t.price - prev).abs());
            self.tr.push((t.price - prev).abs());
        }
        self.prev = Some(t.price);
        self.prices.push(t.price);
        if self.prices.len() != self.p {
            return None;
        }
        let tr: f64 = self.tr.iter().sum();
        (tr != 0.).then(|| self.vm.iter().sum::<f64>() / tr)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[2.5]).is_err());
    }
    #[test]
    fn steady() {
        let mut x = create(&[2.]).unwrap();
        x.on_trade(&t(1.));
        assert_eq!(x.on_trade(&t(2.)), Some(1.));
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
