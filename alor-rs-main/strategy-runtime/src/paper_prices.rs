//! Paper equivalents of command_consumer's price normalization contract.
//! Keep these aligned with the gateway; do not round model bars or Market fills.

use alor_protocol::Side;

pub(crate) fn limit_price(price: f64, step: f64, side: Side) -> f64 {
    if step <= 0.0 {
        return price;
    }
    let scaled = price / step;
    let adjusted = match side {
        Side::Buy => scaled.floor(),
        Side::Sell => scaled.ceil(),
    };
    adjusted * step
}

pub(crate) fn stop_or_replace_price(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return value;
    }
    let normalized = (value / step).round() * step;
    let scale = 10_f64.powi(decimal_places_for_step(step) as i32);
    (normalized * scale).round() / scale
}

fn decimal_places_for_step(step: f64) -> u32 {
    let mut scaled = step.abs();
    for decimals in 0..=8 {
        if (scaled - scaled.round()).abs() <= 1e-9 {
            return decimals;
        }
        scaled *= 10.0;
    }
    8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_rounding_matches_gateway_side_policy() {
        for (raw, step, buy, sell) in [
            (2322.34, 0.5, 2322.0, 2322.5),
            (2310.9, 0.5, 2310.5, 2311.0),
            (2322.5, 0.5, 2322.5, 2322.5),
            (81.513, 0.01, 81.51, 81.52),
            (91393.0, 10.0, 91390.0, 91400.0),
        ] {
            assert!((limit_price(raw, step, Side::Buy) - buy).abs() < 1e-9);
            assert!((limit_price(raw, step, Side::Sell) - sell).abs() < 1e-9);
        }
    }

    #[test]
    fn stop_and_replace_rounding_matches_gateway_nearest_tick_policy() {
        for (raw, expected) in [
            (2386.25, 2386.5),
            (2437.75, 2438.0),
            (2420.52, 2420.5),
            (2438.25, 2438.5),
        ] {
            assert_eq!(stop_or_replace_price(raw, 0.5), expected);
        }
        assert_eq!(stop_or_replace_price(71.93 + 0.01, 0.01), 71.94);
    }

    #[test]
    fn nonpositive_step_preserves_gateway_passthrough() {
        for step in [0.0, -0.5] {
            assert_eq!(limit_price(2322.34, step, Side::Buy), 2322.34);
            assert_eq!(limit_price(2322.34, step, Side::Sell), 2322.34);
            assert_eq!(stop_or_replace_price(2322.34, step), 2322.34);
        }
    }
}
