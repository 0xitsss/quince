use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, Ema, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_dema",
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
    a: Ema,
    b: Ema,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(p)?;
    if p[0].fract() != 0. {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "period must be integral",
        });
    }
    let n = p[0] as usize;
    Ok(Box::new(Value {
        a: Ema::new(n),
        b: Ema::new(n),
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        let a = self.a.update(t.price);
        let b = self.b.update(a);
        Some(2. * a - b)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[2.]).is_ok());
        assert!(create(&[2.1]).is_err());
    }
    #[test]
    fn first() {
        let mut x = create(&[2.]).unwrap();
        assert_eq!(x.on_trade(&t(5.)), Some(5.));
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
