use serde_json::json;
use uuid::Uuid;

use crate::config::AlorGatewayConfig;

pub fn build_bars_subscribe(
    cfg: &AlorGatewayConfig,
    symbol: &str,
    access_token: &str,
    from_ts: i64,
    skip_history: bool,
) -> (String, String) {
    let guid = new_guid();
    let msg = json!({
        "opcode": "BarsGetAndSubscribe",
        "exchange": cfg.exchange,
        "code": symbol,
        "instrumentGroup": cfg.instrument_group,
        "tf": cfg.tf_sec,
        "from": from_ts,
        "skipHistory": skip_history,
        "splitAdjust": cfg.split_adjust,
        "format": cfg.format,
        "frequency": cfg.frequency_ms,
        "guid": guid,
        "token": access_token,
    });
    (guid, msg.to_string())
}

pub fn build_positions_subscribe(
    cfg: &AlorGatewayConfig,
    access_token: &str,
    skip_history: bool,
) -> (String, String) {
    let guid = new_guid();
    let msg = json!({
        "opcode": "PositionsGetAndSubscribeV2",
        "portfolio": cfg.portfolio,
        "exchange": cfg.exchange,
        "skipHistory": skip_history,
        "guid": guid,
        "token": access_token,
    });
    (guid, msg.to_string())
}

pub fn build_orders_subscribe(
    cfg: &AlorGatewayConfig,
    access_token: &str,
    skip_history: bool,
) -> (String, String) {
    let guid = new_guid();
    let msg = json!({
        "opcode": "OrdersGetAndSubscribeV2",
        "portfolio": cfg.portfolio,
        "exchange": cfg.exchange,
        "skipHistory": skip_history,
        "guid": guid,
        "token": access_token,
    });
    (guid, msg.to_string())
}

pub fn build_trades_subscribe(
    cfg: &AlorGatewayConfig,
    access_token: &str,
    skip_history: bool,
) -> (String, String) {
    let guid = new_guid();
    let msg = json!({
        "opcode": "TradesGetAndSubscribeV2",
        "portfolio": cfg.portfolio,
        "exchange": cfg.exchange,
        "skipHistory": skip_history,
        "guid": guid,
        "token": access_token,
    });
    (guid, msg.to_string())
}

fn new_guid() -> String {
    Uuid::new_v4().to_string()
}
