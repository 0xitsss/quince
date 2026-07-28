// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Cumulative net price displacement divided by cumulative absolute movement.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_efficiency_ratio",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    first: Option<f64>,
    previous: Option<f64>,
    path: f64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !p.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "accepts no parameters",
        });
    }
    Ok(Box::new(Indicator {
        first: None,
        previous: None,
        path: 0.,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() {
            return None;
        }
        let first = *self.first.get_or_insert(t.price);
        match self.previous.replace(t.price) {
            None => None,
            Some(old) => {
                self.path += (t.price - old).abs();
                if self.path == 0. {
                    return Some(0.);
                }
                let v = (t.price - first).abs() / self.path;
                v.is_finite().then_some(v)
            }
        }
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
    fn straight_line_is_efficient() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(1.));
        i.on_trade(&t(2.));
        assert_eq!(i.on_trade(&t(3.)), Some(1.));
    }
}
