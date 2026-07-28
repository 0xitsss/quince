// SPDX-FileCopyrightText: 2026 0xitsss
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Number of consecutive non-zero price ticks in the latest direction.
use quince_core::types::Trade;
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};
static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_tick_run_length",
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
    direction: f64,
    run: u64,
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
        direction: 0.,
        run: 0,
    }))
}
impl CustomIndicator for Indicator {
    fn on_trade(&mut self, t: &Trade) -> Option<f64> {
        if !t.price.is_finite() {
            return None;
        }
        let old = self.previous.replace(t.price)?;
        let d = (t.price - old).signum();
        if d == 0. {
            return Some(0.);
        }
        if d == self.direction {
            self.run = self.run.saturating_add(1);
        } else {
            self.direction = d;
            self.run = 1;
        }
        Some(self.run as f64)
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
    fn tracks_directional_run() {
        let mut i = create(&[]).unwrap();
        i.on_trade(&t(1.));
        i.on_trade(&t(2.));
        assert_eq!(i.on_trade(&t(3.)), Some(2.));
        assert_eq!(i.on_trade(&t(2.)), Some(1.));
    }
}
