//! Output DTOs and validation helpers.

pub mod digest;
pub mod emit;
pub mod geom;
pub mod model;
pub mod pipeline;
pub mod provenance;
pub mod report;
pub mod tar;

pub use emit::{
    TopologyArtifacts, compile_network_lfca, compile_network_lfca_with_infeasibility_report,
};

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
