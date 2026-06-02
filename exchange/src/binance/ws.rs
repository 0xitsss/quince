use crate::r#trait::{ExchangeError, Result, StreamMsg};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

const CHAN_CAP: usize = 64;
const MAX_PENDING: usize = 32;

pub struct WsClient {
    pub req_tx: mpsc::Sender<WsRequest>,
}

pub struct WsRequest {
    pub method: String,
    pub params: Map<String, Value>,
    pub response_tx: oneshot::Sender<Result<Value>>,
}

fn sign_params(
    api_key: &str,
    secret_key: &str,
    params: &mut Map<String, Value>,
) {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    params.insert(
        "apiKey".into(),
        Value::String(api_key.to_string()),
    );
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string();
    params.insert("timestamp".into(), Value::String(ts));

    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort_unstable();
    let query = keys
        .iter()
        .filter(|k| **k != "signature")
        .map(|k| {
            let v = &params[*k];
            let vs = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => v.to_string(),
            };
            format!("{}={}", k, vs)
        })
        .collect::<Vec<_>>()
        .join("&");

    let mut mac = Hmac::<Sha256>::new_from_slice(secret_key.as_bytes()).unwrap();
    mac.update(query.as_bytes());
    params.insert(
        "signature".into(),
        Value::String(hex::encode(mac.finalize().into_bytes())),
    );
}

pub struct BinanceWs {
    url: String,
    api_key: String,
    secret_key: String,
}

impl BinanceWs {
    pub fn new(api_key: &str, secret_key: &str, testnet: bool) -> Self {
        let url = if testnet {
            "wss://testnet.binancefuture.com/ws-fapi/v1"
        } else {
            "wss://ws-fapi.binance.com/ws-fapi/v1"
        };
        Self {
            url: url.to_string(),
            api_key: api_key.to_string(),
            secret_key: secret_key.to_string(),
        }
    }

    pub async fn connect(
        &self,
        symbols: &[String],
    ) -> Result<(WsClient, mpsc::Receiver<StreamMsg>)> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.url)
            .await
            .map_err(|e| ExchangeError::Ws(e.to_string()))?;

        let (mut writer, mut reader) = ws_stream.split();
        let (market_tx, market_rx) = mpsc::channel(1024);
        let (req_tx, mut req_rx) = mpsc::channel::<WsRequest>(CHAN_CAP);

        let streams: Vec<String> = symbols
            .iter()
            .flat_map(|s| {
                let s = s.to_lowercase();
                vec![
                    format!("{}@aggTrade", s),
                    format!("{}@depth20@100ms", s),
                    format!("{}@markPrice", s),
                    format!("{}@openInterest", s),
                ]
            })
            .collect();

        let mut subscribe = Map::new();
        subscribe.insert("method".into(), Value::String("SUBSCRIBE".into()));
        subscribe.insert(
            "params".into(),
            Value::Array(streams.into_iter().map(Value::String).collect()),
        );
        subscribe.insert("id".into(), Value::Number(0.into()));
        let _ = writer
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&subscribe).unwrap(),
            ))
            .await;

        let api_key = self.api_key.clone();
        let secret_key = self.secret_key.clone();

        tokio::spawn(async move {
            let mut pending: HashMap<u64, oneshot::Sender<Result<Value>>> =
                HashMap::with_capacity(MAX_PENDING);
            let next_id: AtomicU64 = AtomicU64::new(1);

            loop {
                tokio::select! {
                    Some(Ok(msg)) = reader.next() => {
                        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                if let Some(id_val) = json.get("id") {
                                    let id = match id_val {
                                        Value::Number(n) => n.as_u64().unwrap_or(u64::MAX),
                                        Value::String(s) => s.parse().unwrap_or(u64::MAX),
                                        _ => continue,
                                    };
                                    if let Some(sender) = pending.remove(&id) {
                                        if json.get("error").is_some() {
                                            let err_msg = json["error"]["msg"].as_str().unwrap_or("ws error");
                                            let _ = sender.send(Err(ExchangeError::Order(err_msg.into())));
                                        } else {
                                            let _ = sender.send(Ok(json.get("result").cloned().unwrap_or(Value::Null)));
                                        }
                                    }
                                } else if json.get("e").is_some() {
                                    if let Some(stream_msg) = super::types::parse_ws_msg(&text) {
                                        let _ = market_tx.send(stream_msg).await;
                                    }
                                }
                            }
                        }
                    }
                    Some(req) = req_rx.recv() => {
                        let id = next_id.fetch_add(1, Ordering::Relaxed);
                        let mut params = req.params;
                        sign_params(&api_key, &secret_key, &mut params);
                        let mut request = Map::new();
                        request.insert("id".into(), Value::Number(id.into()));
                        request.insert("method".into(), Value::String(req.method));
                        request.insert("params".into(), Value::Object(params));

                        let payload = serde_json::to_string(&request).unwrap();
                        if let Err(e) = writer.send(
                            tokio_tungstenite::tungstenite::Message::Text(payload),
                        ).await {
                            let _ = req.response_tx.send(Err(ExchangeError::Ws(e.to_string())));
                            continue;
                        }
                        pending.insert(id, req.response_tx);
                    }
                    else => break,
                }
            }
        });

        Ok((WsClient { req_tx }, market_rx))
    }
}
