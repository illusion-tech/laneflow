#![doc = include_str!("../README.md")]

//! current JSON 运行时入口已由 #301 拆除。本 crate 不再把 traffic JSON 正规化为
//! 可运行世界。

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
