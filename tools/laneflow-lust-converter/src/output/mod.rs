//! Output DTOs and validation helpers.

pub mod digest;
pub mod emit;
pub mod geom;
pub mod model;
pub mod pipeline;
pub mod provenance;
pub mod report;
pub mod tar;

pub use digest::hex_sha256;
pub use emit::{
    TopologyArtifacts, TopologyCounts, compile_network_lfca,
    compile_network_lfca_with_infeasibility_report,
};
pub use geom::{
    BudgetOutcome, InfeasibilityDiagnosis, InfeasibilityMechanism, InfeasibilityReport,
    ReportSource,
};
pub use pipeline::{ConvertOutputPaths, convert_with_config};
pub use provenance::{
    BuildInvocation, BuildProvenanceInput, LicenseArtifacts, RawOutputDigests, ReleaseAssetUrls,
    SemanticConfig, SemanticProvenanceInput, build_build_provenance, build_semantic_provenance,
    embedded_notice_bytes, embedded_odbl_bytes,
};
pub use report::{ConversionReportInput, build_conversion_report};
pub use tar::{TarMember, write_deterministic_ustar};

/// Serialize `value` as pretty JSON with a trailing newline.
pub(crate) fn json_bytes<T: serde::Serialize>(
    document: &'static str,
    value: &T,
) -> crate::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|source| crate::Error::Json { document, source })?;
    bytes.push(b'\n');
    Ok(bytes)
}
