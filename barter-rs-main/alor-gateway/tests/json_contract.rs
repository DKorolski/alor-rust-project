use std::fs;

#[test]
fn json_fixtures_parse() {
    let fixtures = [
        "tests/fixtures/bars_ack.json",
        "tests/fixtures/positions_ack.json",
        "tests/fixtures/orders_ack.json",
    ];
    for fixture in fixtures {
        let contents = fs::read_to_string(fixture)
            .unwrap_or_else(|err| panic!("failed to read {}: {}", fixture, err));
        let value: serde_json::Value = serde_json::from_str(&contents)
            .unwrap_or_else(|err| panic!("failed to parse {}: {}", fixture, err));
        assert!(value.is_object(), "fixture {} should be object", fixture);
    }
}
