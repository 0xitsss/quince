// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Online excess population kurtosis of simple returns.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_return_kurtosis",
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
    n: f64,
    mean: f64,
    m2: f64,
    m3: f64,
    m4: f64,
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
        n: 0.,
        mean: 0.,
        m2: 0.,
        m3: 0.,
        m4: 0.,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() || t.price <= 0. {
            return None;
        }
        let old = self.previous.replace(t.price)?;
        let x = t.price / old - 1.;
        if !x.is_finite() {
            return None;
        }
        let n1 = self.n;
        self.n += 1.;
        let d = x - self.mean;
        let dn = d / self.n;
        let dn2 = dn * dn;
        let term = d * dn * n1;
        self.m4 += term * dn2 * (self.n * self.n - 3. * self.n + 3.) + 6. * dn2 * self.m2
            - 4. * dn * self.m3;
        self.m3 += term * dn * (self.n - 2.) - 3. * dn * self.m2;
        self.m2 += term;
        self.mean += dn;
        if self.n < 2. || self.m2 <= 0. {
            return Some(0.);
        }
        let v = self.n * self.m4 / (self.m2 * self.m2) - 3.;
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
    fn finite_after_returns() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(100.));
        i.on_trade(&t(110.));
        assert!(i.on_trade(&t(100.)).unwrap().is_finite());
    }
}
