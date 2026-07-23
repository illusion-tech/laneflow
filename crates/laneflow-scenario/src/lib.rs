#![doc = include_str!("../README.md")]

/// v0.8 直行信号化走廊的可选 runtime support。
pub mod signalized_corridor;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_scenario_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-scenario");
    }
}
