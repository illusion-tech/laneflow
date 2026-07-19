#![doc = include_str!("../README.md")]

mod error;
mod primitives;
mod registry;

pub use error::{SpatialAxis, SpatialError};
pub use primitives::{
    CANONICAL_FRAME_ID_PATTERN, CANONICAL_POINT_COMPONENT_MAX_METERS,
    CANONICAL_POINT_COMPONENT_MIN_METERS, CanonicalFrameId, CanonicalPoint3F32,
    CanonicalUnitVector3F32, CanonicalVector3F32,
};
pub use registry::SpatialRegistry;

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_spatial_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-spatial");
    }
}
