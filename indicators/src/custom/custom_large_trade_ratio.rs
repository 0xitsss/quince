// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Share of trades whose quantity is at least a configured threshold.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};
static PARAMS: &[IndicatorParameter] = &[IndicatorParameter {
    name: "threshold",
    min: 0.000000001,
    max: 1e15,
}];
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_large_trade_ratio",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: PARAMS,
};
pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};
struct Indicator {
    threshold: f64,
    large: u64,
    total: u64,
}
fn create(p: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if p.len() != 1 {
        return Err(CustomIndicatorError::InvalidParameterCount {
            indicator: DESCRIPTOR.name,
            expected: 1,
            actual: p.len(),
        });
    }
    Ok(Box::new(Indicator {
        threshold: p[0],
        large: 0,
        total: 0,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.qty.is_finite() || t.qty <= 0. {
            return None;
        }
        self.total += 1;
        if t.qty >= self.threshold {
            self.large += 1;
        }
        Some(self.large as f64 / self.total as f64)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn t(q: f64) -> Trade {
        Trade {
            price: 1.,
            qty: q,
            side: quince_core::types::Side::Buy,
            time: Utc::now(),
            trade_id: 1,
        }
    }
    #[test]
    fn counts_large_trades() {
        let mut i = create(&[5.]).unwrap();
        i.on_trade(&t(2.));
        assert_eq!(i.on_trade(&t(5.)), Some(0.5));
    }
}
