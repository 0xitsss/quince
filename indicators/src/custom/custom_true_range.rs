use quince_core::types::Trade;
use quince_indicators::{CustomIndicator,CustomIndicatorError,CustomIndicatorRegistration,IndicatorDescriptor,IndicatorInput,IndicatorOutput};
static D:IndicatorDescriptor=IndicatorDescriptor{name:"custom_true_range",input:IndicatorInput::Trade,output:IndicatorOutput::ScalarF64,parameters:&[]}; pub static REGISTRATION:CustomIndicatorRegistration=CustomIndicatorRegistration{descriptor:&D,create};
struct I{prev:Option<f64>} fn create(p:&[f64])->Result<Box<dyn CustomIndicator>,CustomIndicatorError>{if p.is_empty(){Ok(Box::new(I{prev:None}))}else{Err(CustomIndicatorError::Construction{indicator:D.name,reason:"accepts no parameters"})}}
impl CustomIndicator for I{fn on_trade(&mut self,t:&Trade)->Option<f64>{let v=self.prev.map(|p|(t.price-p).abs()).unwrap_or(0.0);self.prev=Some(t.price);Some(v)}}
#[cfg(test)] mod tests{use super::*;use chrono::Utc;use quince_core::types::Side;fn t(p:f64)->Trade{Trade{price:p,qty:1.,time:Utc::now(),side:Side::Buy,trade_id:1}}#[test]fn range(){let mut i=create(&[]).unwrap();i.on_trade(&t(10.));assert_eq!(i.on_trade(&t(12.)),Some(2.));}}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
