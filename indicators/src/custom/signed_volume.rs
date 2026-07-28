// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Example custom indicator discovered automatically by `build.rs`.

use quince_core::types::{Side, Trade};
use quince_indicators::{
    CustomIndicator, CustomIndicatorError, CustomIndicatorRegistration, IndicatorDescriptor,
    IndicatorInput, IndicatorOutput,
};

static DESCRIPTOR: IndicatorDescriptor = IndicatorDescriptor {
    name: "signed_volume",
    input: IndicatorInput::Trade,
    output: IndicatorOutput::ScalarF64,
    parameters: &[],
};

pub static REGISTRATION: CustomIndicatorRegistration = CustomIndicatorRegistration {
    descriptor: &DESCRIPTOR,
    create,
};

struct SignedVolume {
    value: f64,
}

fn create(params: &[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError> {
    if !params.is_empty() {
        return Err(CustomIndicatorError::Construction {
            indicator: DESCRIPTOR.name,
            reason: "signed_volume accepts no parameters",
        });
    }
    Ok(Box::new(SignedVolume { value: 0.0 }))
}

impl CustomIndicator for SignedVolume {
    fn on_trade(&mut self, trade: &Trade) -> Option<f64> {
        self.value += if trade.side == Side::Buy { trade.qty } else { -trade.qty };
        Some(self.value)
    }
}
