use std::collections::HashMap;

use alor_types::{PositionSnapshot, PositionsSnapshot};

#[test]
fn positions_snapshot_roundtrip_json() {
    let mut positions = HashMap::new();
    positions.insert(
        "SBER".to_string(),
        PositionSnapshot {
            symbol: "SBER".to_string(),
            qty: 2.0,
            avg_price: 312.55,
            ts_utc: 1_739_000_000,
        },
    );

    let snapshot = PositionsSnapshot { positions };
    let json = serde_json::to_string(&snapshot).expect("serialize positions snapshot");
    let decoded: PositionsSnapshot =
        serde_json::from_str(&json).expect("deserialize positions snapshot");

    assert_eq!(snapshot, decoded);
}
