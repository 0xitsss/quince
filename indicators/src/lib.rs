pub mod flow;
pub mod ma;
pub mod oscillator;
pub mod structure;
pub mod volatility;

pub use flow::*;
pub use ma::*;
pub use oscillator::*;
pub use structure::*;
pub use volatility::*;

#[derive(Debug, Clone, Copy)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Candle {
    pub fn new(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            open,
            high,
            low,
            close,
            volume,
        }
    }

    pub fn typical(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    pub fn from_trade(price: f64, volume: f64) -> Self {
        Self {
            open: price,
            high: price,
            low: price,
            close: price,
            volume,
        }
    }
}
