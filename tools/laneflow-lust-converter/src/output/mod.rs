//! Output DTOs and validation helpers.

pub mod digest;
pub mod emit;
pub mod model;
pub mod pipeline;
pub mod provenance;
pub mod report;
pub mod tar;

pub use emit::{TopologyArtifacts, finish_topology_artifacts, json_bytes};
pub use pipeline::{ConvertOutputPaths, convert_with_config};
pub use provenance::{
    BuildInvocation, BuildProvenanceInput, LicenseArtifacts, RawOutputDigests, ReleaseAssetUrls,
    SemanticProvenanceInput, build_build_provenance, build_semantic_provenance,
    embedded_notice_bytes, embedded_odbl_bytes,
};
pub use report::{ConversionReportInput, build_conversion_report};
pub use tar::{TarMember, write_deterministic_ustar};
