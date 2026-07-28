// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Online logistic regression over trade log-returns.

use quince_core::types::{Side, Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput, IndicatorParameter,
};

static PARAMETERS: [IndicatorParameter; 2] = [
    IndicatorParameter {
        name: "learning_rate",
        min: 0.000_001,
        max: 1.0,
    },
    IndicatorParameter {
        name: "l2",
        min: 0.0,
        max: 1.0,
    },
];

static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "custom_logistic_regression",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &PARAMETERS,
};

pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};

struct LogisticRegression {
    learning_rate: f64,
    l2: f64,
    previous_price: Option<f64>,
    weight: f64,
    bias: f64,
}

fn create(params: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    REGISTRATION.validate_params(params)?;
    Ok(Box::new(LogisticRegression {
        learning_rate: params[0],
        l2: params[1],
        previous_price: None,
        weight: 0.0,
        bias: 0.0,
    }))
}

impl CustomIndicator for LogisticRegression {
    fn on_trade(&mut self, trade: &Trade) -> Option<f64> {
        let previous = self.previous_price.replace(trade.price)?;
        let feature = (trade.price / previous).ln();
        if !feature.is_finite() {
            return None;
        }
        let score = self.bias + self.weight * feature;
        // Stable sigmoid avoids overflow from a long-running online model.
        let probability = if score >= 0.0 {
            1.0 / (1.0 + (-score).exp())
        } else {
            let exp = score.exp();
            exp / (1.0 + exp)
        };
        let label = if trade.side == Side::Buy { 1.0 } else { 0.0 };
        let error = label - probability;
        self.weight += self.learning_rate * (error * feature - self.l2 * self.weight);
        self.bias += self.learning_rate * error;
        probability.is_finite().then_some(probability)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(price: f64, side: Side) -> Trade {
        Trade {
            price,
            qty: 1.0,
            time: chrono::Utc::now(),
            side,
            trade_id: 1,
        }
    }

    #[test]
    fn online_logistic_regression_returns_a_probability_after_warmup() {
        let mut indicator = create(&[0.1, 0.01]).expect("valid parameters");
        assert_eq!(indicator.on_trade(&trade(100.0, Side::Buy)), None);
        let probability = indicator
            .on_trade(&trade(101.0, Side::Buy))
            .expect("second observation has a return feature");
        assert!((0.0..=1.0).contains(&probability));
    }

    #[test]
    fn online_logistic_regression_rejects_invalid_learning_rate() {
        assert!(create(&[0.0, 0.1]).is_err());
    }
}
