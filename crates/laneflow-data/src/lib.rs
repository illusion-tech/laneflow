#![doc = include_str!("../README.md")]

pub use laneflow_current_source::CURRENT_TRAFFIC_FORMAT_VERSION as CURRENT_FORMAT_VERSION;
pub use laneflow_current_source::{
    CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION,
    SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE,
};

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_data_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-data");
    }
}
