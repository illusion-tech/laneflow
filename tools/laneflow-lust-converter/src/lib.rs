//! LuST Scenario v2.0 source/static converter (#253).
//!
//! Current delivery: pinned source verification, SUMO net → topology-only
//! Traffic/Spatial/Manifest conversion (empty junctions/signals/profiles/routes).
//! Full static bundle (junctions, signals, profiles, DUE routes, provenance, tar)
//! lands in follow-up commits on the same Delivery PR.

mod config;
mod convert;
mod error;
mod output;
mod source;
mod sumo;

pub use config::{LustConverterConfig, load_config};
pub use convert::{TopologyConvertOptions, convert_network_topology};
pub use error::{Error, Result};
pub use output::TopologyArtifacts;
pub use source::{
    LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES, PinnedSourceFile,
    VerifiedSourceFile, VerifiedSourceSet, verify_source_dir,
};
pub use sumo::{
    ExactDecimal, LUST_CONV_BOUNDARY, LUST_FRAME_ID, LUST_NET_OFFSET, SUMO_ID_PREFIX, SumoNetwork,
    parse_sumo_network_xml,
};

use std::path::Path;

/// Verify the pinned LuST source set under `source_dir`.
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    verify_source_dir(source_dir)
}

/// Convert topology-only packages from an already-parsed SUMO network.
pub fn convert_topology_from_network(
    network: &SumoNetwork,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology(network, options)
}

/// Convert topology-only packages from SUMO network XML text.
pub fn convert_topology_from_xml(
    xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(xml)?;
    convert_network_topology(&network, options)
}

/// Run convert after verifying pinned source; full static conversion is not ready yet.
pub fn convert(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    let _verified = verify_source_dir(&config.source_dir)?;
    let _ = config.output_dir;
    Err(Error::StaticConversionNotImplemented)
}
