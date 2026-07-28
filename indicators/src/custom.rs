// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Native, compile-time custom-indicator extension API.
//!
//! Put one Rust source file in `src/custom/`. The build script discovers it at
//! compile time and adds its [`CustomIndicatorRegistration`] to the registry.
//! Dynamic loading is deliberately unsupported: every plugin is reviewed,
//! compiled, and linked into the Quince binary.

use quince_core::types::Trade;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Market-event format accepted by an indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorInput {
    /// The indicator is updated for every public trade.
    Trade,
}

/// Output format exposed to QFL through `quince.get("<name>")`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorOutput {
    /// One finite `f64` value; `None` from the trait means warm-up is incomplete.
    ScalarF64,
}

/// A named numeric parameter accepted by a custom indicator.
#[derive(Debug, Clone, Copy)]
pub struct IndicatorParameter {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
}

/// Immutable metadata declared by every custom indicator.
#[derive(Debug, Clone, Copy)]
pub struct IndicatorDescriptor {
    /// QFL name used in `@using`, restricted to lowercase ASCII identifiers.
    pub name: &'static str,
    /// Input event format. Trade is the only supported format in v1.
    pub input: IndicatorInput,
    /// QFL-visible output format. v1 supports one scalar value per indicator.
    pub output: IndicatorOutput,
    /// Ordered numeric parameters used after the name in `@using`.
    pub parameters: &'static [IndicatorParameter],
}

/// Construction or validation failure for a custom indicator.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomIndicatorError {
    UnknownIndicator(String),
    InvalidParameterCount {
        indicator: &'static str,
        expected: usize,
        actual: usize,
    },
    InvalidParameter {
        indicator: &'static str,
        parameter: &'static str,
        value: f64,
        min: f64,
        max: f64,
    },
    Construction {
        indicator: &'static str,
        reason: &'static str,
    },
}

impl Display for CustomIndicatorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownIndicator(name) => write!(f, "unknown indicator `{name}`"),
            Self::InvalidParameterCount {
                indicator,
                expected,
                actual,
            } => write!(
                f,
                "indicator `{indicator}` expects {expected} parameters, got {actual}"
            ),
            Self::InvalidParameter {
                indicator,
                parameter,
                value,
                min,
                max,
            } => write!(
                f,
                "indicator `{indicator}` parameter `{parameter}`={value} is outside [{min}, {max}]"
            ),
            Self::Construction { indicator, reason } => {
                write!(f, "indicator `{indicator}` construction failed: {reason}")
            }
        }
    }
}

impl Error for CustomIndicatorError {}

/// Native indicator implementation. `on_trade` must not allocate or block.
pub trait CustomIndicator: Send {
    /// Returns the current scalar value, or `None` until the indicator warms up.
    fn on_trade(&mut self, trade: &Trade) -> Option<f64>;
}

/// Compile-time registration emitted by a custom-indicator source file.
#[derive(Clone, Copy)]
pub struct CustomIndicatorRegistration {
    pub descriptor: &'static IndicatorDescriptor,
    pub create: CustomIndicatorFactory,
}

/// Factory signature used by the generated custom-indicator registry.
pub type CustomIndicatorFactory =
    fn(&[f64]) -> Result<Box<dyn CustomIndicator>, CustomIndicatorError>;

impl CustomIndicatorRegistration {
    /// Validates the generic manifest contract before calling plugin code.
    pub fn validate_params(&self, params: &[f64]) -> Result<(), CustomIndicatorError> {
        let expected = self.descriptor.parameters.len();
        if params.len() != expected {
            return Err(CustomIndicatorError::InvalidParameterCount {
                indicator: self.descriptor.name,
                expected,
                actual: params.len(),
            });
        }
        for (value, spec) in params.iter().zip(self.descriptor.parameters) {
            if !value.is_finite() || *value < spec.min || *value > spec.max {
                return Err(CustomIndicatorError::InvalidParameter {
                    indicator: self.descriptor.name,
                    parameter: spec.name,
                    value: *value,
                    min: spec.min,
                    max: spec.max,
                });
            }
        }
        Ok(())
    }
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/custom_indicator_registry.rs"));
}

/// Finds a compile-time registered custom indicator by its QFL name.
pub fn custom_indicator(name: &str) -> Option<&'static CustomIndicatorRegistration> {
    custom_indicators()
        .iter()
        .find(|registration| registration.descriptor.name == name)
}

/// All custom indicators linked into this Quince build, in deterministic
/// filename order. This is intended for startup validation and tooling only.
pub fn custom_indicators() -> &'static [CustomIndicatorRegistration] {
    generated::registrations()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_contract_has_unique_well_formed_native_indicators() {
        // `signed_volume` is the original reference plugin and the 50
        // `custom_*.rs` modules form the first native-indicator catalogue.
        assert_eq!(custom_indicators().len(), 52);

        let mut names = HashSet::with_capacity(custom_indicators().len());
        for registration in custom_indicators() {
            let descriptor = registration.descriptor;
            assert_eq!(descriptor.input, IndicatorInput::Trade);
            assert_eq!(descriptor.output, IndicatorOutput::ScalarF64);
            assert!(
                !descriptor.name.is_empty()
                    && descriptor.name.bytes().enumerate().all(|(index, byte)| {
                        (index == 0 && byte.is_ascii_lowercase())
                            || (index > 0
                                && (byte.is_ascii_lowercase()
                                    || byte.is_ascii_digit()
                                    || byte == b'_'))
                    }),
                "invalid custom indicator name `{}`",
                descriptor.name
            );
            assert!(
                names.insert(descriptor.name),
                "duplicate `{}`",
                descriptor.name
            );
            for parameter in descriptor.parameters {
                assert!(!parameter.name.is_empty());
                assert!(parameter.min.is_finite() && parameter.max.is_finite());
                assert!(parameter.min <= parameter.max, "{}", descriptor.name);
            }
        }
    }
}
