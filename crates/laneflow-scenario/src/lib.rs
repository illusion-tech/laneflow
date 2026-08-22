#![doc = include_str!("../README.md")]

pub mod signalized_corridor;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_scenario_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-scenario");
    }
}
