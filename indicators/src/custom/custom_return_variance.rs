// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Online population variance of simple trade-to-trade returns.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_return_variance",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    previous: Option<f64>,
    n: u64,
    mean: f64,
    m2: f64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator {
        previous: None,
        n: 0,
        mean: 0.,
        m2: 0.,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() || t.price <= 0. {
            return None;
        }
        let old = self.previous.replace(t.price)?;
        if old <= 0. {
            return None;
        }
        let r = t.price / old - 1.;
        if !r.is_finite() {
            return None;
        }
        self.n += 1;
        let d = r - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (r - self.mean);
        let v = self.m2 / self.n as f64;
        v.is_finite().then_some(v)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(p: f64) -> Trade {
        Trade {
            price: p,
            qty: 1.,
            side: quince_core::types::Side::Buy,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn variance_is_nonnegative() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(100.));
        i.on_trade(&t(110.));
        assert!(i.on_trade(&t(100.)).unwrap() > 0.);
    }
}
