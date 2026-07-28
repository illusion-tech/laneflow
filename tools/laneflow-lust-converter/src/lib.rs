//! LuST Scenario v2.0 source/static converter (#253).
//!
//! Delivers pinned source verification, static Traffic/Spatial conversion
//! (Junction/Movement/ManeuverPath/Signals/profiles/DUE routes/population),
//! conversion report, deterministic tar bundles, and semantic/build provenance.

mod config;
mod convert;
mod error;
mod output;
mod source;
mod sumo;

pub use config::{LustConverterConfig, load_config, load_config_with_bytes};
pub use convert::{
    LUST_PASSENGER_VTYPE_IDS, POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS,
    POPULATION_DEPART_START_SECONDS, POPULATION_SELECTED_COUNT, PopulationRecord,
    StaticConversionArtifacts, TopologyConvertOptions, convert_network_topology,
    convert_network_topology_with_tll, select_passenger_vtypes, select_population,
};
pub use error::{Error, Result};
pub use output::{
    BuildInvocation, BuildProvenanceInput, ConversionReportInput, ConvertOutputPaths,
    LicenseArtifacts, RawOutputDigests, ReleaseAssetUrls, SemanticProvenanceInput, TarMember,
    TopologyArtifacts, build_build_provenance, build_conversion_report, build_semantic_provenance,
    convert_with_config, embedded_notice_bytes, embedded_odbl_bytes, write_deterministic_ustar,
};
pub use source::{
    LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES, PinnedSourceFile,
    VerifiedSourceFile, VerifiedSourceSet, verify_source_dir,
};
pub use sumo::{
    DueVehicle, ExactDecimal, LUST_CONV_BOUNDARY, LUST_FRAME_ID, LUST_NET_OFFSET, SUMO_ID_PREFIX,
    SumoNetwork, SumoVType, parse_due_routes_xml, parse_sumo_network_xml, parse_tll_static_xml,
    parse_vtypes_xml,
};

use std::path::Path;

use crate::convert::{
    convert_network_topology_with_tll_and_profiles, convert_static_with_due,
    convert_vehicle_profiles,
};
use crate::output::convert_with_config as run_convert;

/// Verify the pinned LuST source set under `source_dir`.
pub fn verify_source(source_dir: &Path) -> Result<VerifiedSourceSet> {
    verify_source_dir(source_dir)
}

/// Convert topology packages from an already-parsed SUMO network (no tll/profiles).
pub fn convert_topology_from_network(
    network: &SumoNetwork,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology(network, options)
}

/// Convert topology packages from SUMO network XML text (no tll/profiles).
pub fn convert_topology_from_xml(
    xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(xml)?;
    convert_network_topology(&network, options)
}

/// Convert topology + static signals from network XML and `tll.static.xml` text.
pub fn convert_topology_from_xml_with_tll(
    net_xml: &str,
    tll_xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    convert_network_topology_with_tll(&network, &tll, options)
}

/// Convert topology + signals + passenger profiles from net/tll/vtypes XML.
pub fn convert_topology_from_xml_with_tll_and_vtypes(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    let vtypes = parse_vtypes_xml(vtypes_xml)?;
    let passengers = select_passenger_vtypes(&vtypes)?;
    let profiles = convert_vehicle_profiles(&passengers)?;
    convert_network_topology_with_tll_and_profiles(&network, &tll, &profiles, options)
}

/// Convert topology + DUE routes + population table from net/tll/vtypes/DUE XML.
///
/// `due_xmls` must be the three `local.static.{0,1,2}.rou.xml` texts in order.
pub fn convert_static_from_xml_with_due(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    due_xmls: [&str; 3],
    options: &TopologyConvertOptions,
) -> Result<StaticConversionArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    let vtypes = parse_vtypes_xml(vtypes_xml)?;
    let passengers = select_passenger_vtypes(&vtypes)?;
    let profiles = convert_vehicle_profiles(&passengers)?;
    let mut due_vehicles = Vec::new();
    for (ordinal, xml) in due_xmls.into_iter().enumerate() {
        let file_ordinal = u8::try_from(ordinal).expect("0..2 fits u8");
        due_vehicles.extend(parse_due_routes_xml(xml, file_ordinal)?);
    }
    convert_static_with_due(&network, &tll, &profiles, &due_vehicles, options)
}

/// Verify pinned source and write static/source bundles plus provenance.
pub fn convert(config_path: &Path) -> Result<ConvertOutputPaths> {
    let (config, config_bytes) = load_config_with_bytes(config_path)?;
    run_convert(&config, &config_bytes)
}
