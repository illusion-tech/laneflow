#![doc = include_str!("../README.md")]

pub mod event;
pub mod graph;
pub mod route;
pub mod time;
pub mod vehicle;
pub mod world;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_core_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-core");
    }
}
