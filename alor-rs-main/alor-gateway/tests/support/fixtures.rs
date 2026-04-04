#![allow(dead_code)]

use serde_json::json;

pub fn ack_for_guid(guid: &str) -> String {
    json!({
        "requestGuid": guid,
        "httpCode": 200,
        "message": "Handled successfully",
    })
    .to_string()
}

pub fn bar_message(symbol: &str, close_time: i64, guid: &str) -> String {
    json!({
        "guid": guid,
        "data": {
            "symbol": symbol,
            "time": close_time,
            "open": 100.0,
            "high": 110.0,
            "low": 90.0,
            "close": 105.0,
            "volume": 1.0,
        }
    })
    .to_string()
}
