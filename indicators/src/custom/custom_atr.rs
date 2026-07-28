use quince_core::{ring::RingVec,types::Trade};use quince_indicators::{CustomIndicator,CustomIndicatorError,CustomIndicatorRegistration,IndicatorDescriptor,IndicatorInput,IndicatorOutput,IndicatorParameter};
static P:&[IndicatorParameter]=&[IndicatorParameter{name:"period",min:1.,max:10000.}];static D:IndicatorDescriptor=IndicatorDescriptor{name:"custom_atr",input:IndicatorInput::Trade,output:IndicatorOutput::ScalarF64,parameters:P};pub static REGISTRATION:CustomIndicatorRegistration=CustomIndicatorRegistration{descriptor:&D,create};struct I{p:usize,prev:Option<f64>,v:RingVec}
fn create(x:&[f64])->Result<Box<dyn CustomIndicator>,CustomIndicatorError>{Ok(Box::new(I{p:x[0]as usize,prev:None,v:RingVec::new(x[0]as usize)}))}impl CustomIndicator for I{fn on_trade(&mut self,t:&Trade)->Option<f64>{let tr=self.prev.map(|p|(t.price-p).abs()).unwrap_or(0.);self.prev=Some(t.price);self.v.push(tr);(self.v.len()==self.p).then(||self.v.iter().sum::<f64>()/self.p as f64)}}
#[cfg(test)]mod tests{use super::*;use chrono::Utc;use quince_core::types::Side;fn t(p:f64)->Trade{Trade{price:p,qty:1.,time:Utc::now(),side:Side::Buy,trade_id:1}}#[test]fn warmup(){let mut i=create(&[2.]).unwrap();assert_eq!(i.on_trade(&t(1.)),None);assert_eq!(i.on_trade(&t(3.)),Some(1.));}}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
