#![doc = include_str!("../README.md")]

#[cfg(debug_assertions)]
mod counters;
mod digest;
mod error;
mod parse;
#[doc(hidden)]
pub mod scenario_wire;
mod validate;
#[doc(hidden)]
pub mod wire;

pub use error::{
    CurrentArtifactRole, CurrentDocumentRole, CurrentSourceError, CurrentSourceErrorPayload,
    CurrentSourceIssue, CurrentSourceIssueContext, CurrentSourceIssueParts, CurrentSourcePosition,
    CurrentSourceSpan,
};
pub use validate::{
    CurrentArtifactInput, CurrentDocumentInput, ValidatedCurrentSourceBundle,
    ValidatedCurrentTrafficPackage, validate_scenario_compatible, validate_traffic_compatible,
};
#[doc(hidden)]
pub use validate::{CurrentSourceParts, CurrentTrafficParts};

/// 当前 Traffic package 接受的唯一 format 版本。
pub const CURRENT_TRAFFIC_FORMAT_VERSION: &str = "0.10";

/// 当前 ScenarioManifest 接受的唯一 format 版本。
pub const CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION: &str = "0.1";

/// 当前 SpatialPackage 接受的唯一 format 版本。
pub const CURRENT_SPATIAL_FORMAT_VERSION: &str = "0.1";

/// Traffic package descriptor 的固定 media type。
pub const TRAFFIC_PACKAGE_MEDIA_TYPE: &str = "application/vnd.laneflow.traffic+json";

/// Spatial package descriptor 的固定 media type。
pub const SPATIAL_PACKAGE_MEDIA_TYPE: &str = "application/vnd.laneflow.spatial+json";
