use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, Ema, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_trix",
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
    c: Ema,
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
    let n = x[0] as usize;
    Ok(Box::new(Value {
        a: Ema::new(n),
        b: Ema::new(n),
        c: Ema::new(n),
        prev: None,
    }))
}
impl CustomIndicator for Value {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        let v = self.c.update(self.b.update(self.a.update(t.price)));
        let out = self
            .prev
            .and_then(|p| (p != 0.).then(|| (v - p) / p * 100.));
        self.prev = Some(v);
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation() {
        assert!(create(&[3.]).is_ok());
        assert!(create(&[3.1]).is_err());
    }
    #[test]
    fn warmup() {
        let mut x = create(&[3.]).unwrap();
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
