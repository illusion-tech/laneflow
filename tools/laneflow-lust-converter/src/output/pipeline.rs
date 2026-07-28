//! End-to-end convert packaging: report, licenses, tar, provenance.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Error, Result,
    config::LustConverterConfig,
    convert::TopologyConvertOptions,
    convert_static_from_xml_with_due,
    output::{
        digest::{hex_sha256, sha256_digest},
        provenance::{
            BuildInvocation, BuildProvenanceInput, LicenseArtifacts, RawOutputDigests,
            ReleaseAssetUrls, SemanticProvenanceInput, build_build_provenance,
            build_semantic_provenance, embedded_notice_bytes, embedded_odbl_bytes,
        },
        report::{ConversionReportInput, build_conversion_report},
        tar::{TarMember, write_deterministic_ustar},
    },
    source::{PINNED_SOURCE_FILES, VerifiedSourceSet, verify_source_dir},
    sumo::parse_sumo_network_xml,
};

const TRAFFIC_NAME: &str = "lust-topology.traffic.json";
const SPATIAL_NAME: &str = "lust-topology.spatial.json";
const MANIFEST_NAME: &str = "lust-topology.manifest.json";
const REPORT_NAME: &str = "lust-conversion-report.json";
const POPULATION_NAME: &str = "lust-population.json";
const SOURCE_TAR_NAME: &str = "lust-source.tar";
const STATIC_TAR_NAME: &str = "lust-static.tar";
const SEMANTIC_NAME: &str = "lust-semantic-provenance.json";
const BUILD_NAME: &str = "lust-build-provenance.json";
const LICENSE_NAME: &str = "LICENSE.md";
const ODBL_NAME: &str = "ODbL-1.0.txt";
const NOTICE_NAME: &str = "NOTICE";

/// Paths written by a successful convert.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvertOutputPaths {
    pub output_dir: PathBuf,
    pub traffic: PathBuf,
    pub spatial: PathBuf,
    pub manifest: PathBuf,
    pub conversion_report: PathBuf,
    pub population: PathBuf,
    pub source_tar: PathBuf,
    pub static_tar: PathBuf,
    pub semantic_provenance: PathBuf,
    pub build_provenance: PathBuf,
}

/// Verify pinned source and emit static bundle + provenance under `output_dir`.
pub fn convert_with_config(config: &LustConverterConfig, config_toml_bytes: &[u8]) -> Result<ConvertOutputPaths> {
    let verified = verify_source_dir(&config.source_dir)?;
    convert_verified(config, config_toml_bytes, &verified)
}

fn convert_verified(
    config: &LustConverterConfig,
    config_toml_bytes: &[u8],
    verified: &VerifiedSourceSet,
) -> Result<ConvertOutputPaths> {
    let net_xml = read_verified(verified, "scenario/lust.net.xml")?;
    let tll_xml = read_verified(verified, "scenario/tll.static.xml")?;
    let vtypes_xml = read_verified(verified, "scenario/vtypes.add.xml")?;
    let due0 = read_verified(verified, "scenario/DUERoutes/local.static.0.rou.xml")?;
    let due1 = read_verified(verified, "scenario/DUERoutes/local.static.1.rou.xml")?;
    let due2 = read_verified(verified, "scenario/DUERoutes/local.static.2.rou.xml")?;
    let license_md = read_verified(verified, "LICENSE.md")?;

    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        require_lust_population_count: true,
        traffic_artifact_ref: TRAFFIC_NAME.to_owned(),
        spatial_artifact_ref: SPATIAL_NAME.to_owned(),
        ..TopologyConvertOptions::default()
    };
    let static_artifacts = convert_static_from_xml_with_due(
        &net_xml,
        &tll_xml,
        &vtypes_xml,
        [&due0, &due1, &due2],
        &options,
    )?;

    let network = parse_sumo_network_xml(&net_xml)?;
    let traffic_counts = count_traffic_arrays(&static_artifacts.topology.traffic)?;

    let report = build_conversion_report(&ConversionReportInput {
        external_edge_count: network.external_edge_count() as u64,
        external_lane_count: network.external_lane_count() as u64,
        connection_count: network.connections.len() as u64,
        junction_count: traffic_counts.junctions,
        movement_count: traffic_counts.movements,
        maneuver_path_count: traffic_counts.maneuver_paths,
        route_catalog_count: static_artifacts.route_count as u64,
        vehicle_profile_count: traffic_counts.vehicle_profiles,
        signal_controller_count: traffic_counts.signal_controllers,
        signal_group_count: traffic_counts.signal_groups,
        stop_line_count: traffic_counts.stop_lines,
        maneuver_gate_count: traffic_counts.maneuver_gates,
        population_record_count: static_artifacts.population_record_count as u64,
        require_lust_population_count: true,
        parking_registry_empty: traffic_counts.parking_empty,
        major_minor_green_collapsed: true,
        traffic_bytes: static_artifacts.topology.traffic.clone(),
        spatial_bytes: static_artifacts.topology.spatial.clone(),
        manifest_bytes: static_artifacts.topology.manifest.clone(),
    })?;

    let licenses = LicenseArtifacts {
        license_md: license_md.into_bytes(),
        odbl: embedded_odbl_bytes().to_vec(),
        notice: embedded_notice_bytes().to_vec(),
    };

    let source_tar = build_source_tar(verified, &licenses)?;
    let static_tar = write_deterministic_ustar(&[
        TarMember {
            path: TRAFFIC_NAME.to_owned(),
            contents: static_artifacts.topology.traffic.clone(),
        },
        TarMember {
            path: SPATIAL_NAME.to_owned(),
            contents: static_artifacts.topology.spatial.clone(),
        },
        TarMember {
            path: MANIFEST_NAME.to_owned(),
            contents: static_artifacts.topology.manifest.clone(),
        },
        TarMember {
            path: REPORT_NAME.to_owned(),
            contents: report.clone(),
        },
        TarMember {
            path: LICENSE_NAME.to_owned(),
            contents: licenses.license_md.clone(),
        },
        TarMember {
            path: ODBL_NAME.to_owned(),
            contents: licenses.odbl.clone(),
        },
        TarMember {
            path: NOTICE_NAME.to_owned(),
            contents: licenses.notice.clone(),
        },
    ])?;

    let release_urls = ReleaseAssetUrls {
        source_bundle_url: config.source_bundle_url.clone(),
        static_bundle_url: config.static_bundle_url.clone(),
    };
    let semantic = build_semantic_provenance(&SemanticProvenanceInput {
        config_toml_bytes: config_toml_bytes.to_vec(),
        licenses: licenses.clone(),
        release_urls,
        source_tar: source_tar.clone(),
        static_tar: static_tar.clone(),
        traffic_bytes: static_artifacts.topology.traffic.clone(),
        spatial_bytes: static_artifacts.topology.spatial.clone(),
        manifest_bytes: static_artifacts.topology.manifest.clone(),
        conversion_report_bytes: report.clone(),
        population_bytes: static_artifacts.population.clone(),
    })?;

    let converter_commit = resolve_converter_commit(config)?;
    let cargo_lock_sha256 = hex_sha256(&fs::read(workspace_cargo_lock()).map_err(|source| {
        Error::Io {
            path: workspace_cargo_lock(),
            source,
        }
    })?);
    let build = build_build_provenance(&BuildProvenanceInput {
        converter_commit,
        rust_version: "1.96.0",
        cargo_lock_sha256,
        config_digest: sha256_digest(config_toml_bytes),
        semantic_provenance_digest: sha256_digest(&semantic),
        invocation: BuildInvocation {
            command: "convert",
            require_lust_location_anchors: true,
            require_lust_population_count: true,
            traffic_artifact_ref: TRAFFIC_NAME.to_owned(),
            spatial_artifact_ref: SPATIAL_NAME.to_owned(),
        },
        raw_output_digests: RawOutputDigests {
            traffic: sha256_digest(&static_artifacts.topology.traffic),
            spatial: sha256_digest(&static_artifacts.topology.spatial),
            scenario_manifest: sha256_digest(&static_artifacts.topology.manifest),
            conversion_report: sha256_digest(&report),
            population_table: sha256_digest(&static_artifacts.population),
            source_tar: sha256_digest(&source_tar),
            static_tar: sha256_digest(&static_tar),
        },
    })?;

    fs::create_dir_all(&config.output_dir).map_err(|source| Error::Io {
        path: config.output_dir.clone(),
        source,
    })?;

    let paths = ConvertOutputPaths {
        output_dir: config.output_dir.clone(),
        traffic: config.output_dir.join(TRAFFIC_NAME),
        spatial: config.output_dir.join(SPATIAL_NAME),
        manifest: config.output_dir.join(MANIFEST_NAME),
        conversion_report: config.output_dir.join(REPORT_NAME),
        population: config.output_dir.join(POPULATION_NAME),
        source_tar: config.output_dir.join(SOURCE_TAR_NAME),
        static_tar: config.output_dir.join(STATIC_TAR_NAME),
        semantic_provenance: config.output_dir.join(SEMANTIC_NAME),
        build_provenance: config.output_dir.join(BUILD_NAME),
    };

    write_file(&paths.traffic, &static_artifacts.topology.traffic)?;
    write_file(&paths.spatial, &static_artifacts.topology.spatial)?;
    write_file(&paths.manifest, &static_artifacts.topology.manifest)?;
    write_file(&paths.conversion_report, &report)?;
    write_file(&paths.population, &static_artifacts.population)?;
    write_file(&paths.source_tar, &source_tar)?;
    write_file(&paths.static_tar, &static_tar)?;
    write_file(&paths.semantic_provenance, &semantic)?;
    write_file(&paths.build_provenance, &build)?;
    write_file(&config.output_dir.join(LICENSE_NAME), &licenses.license_md)?;
    write_file(&config.output_dir.join(ODBL_NAME), &licenses.odbl)?;
    write_file(&config.output_dir.join(NOTICE_NAME), &licenses.notice)?;

    Ok(paths)
}

fn build_source_tar(verified: &VerifiedSourceSet, licenses: &LicenseArtifacts) -> Result<Vec<u8>> {
    let mut members = Vec::with_capacity(PINNED_SOURCE_FILES.len() + 2);
    for pinned in PINNED_SOURCE_FILES {
        let file = verified
            .files
            .iter()
            .find(|file| file.relative_path == pinned.relative_path)
            .ok_or_else(|| Error::SumoModel(format!(
                "verified set missing pinned file {}",
                pinned.relative_path
            )))?;
        let contents = fs::read(&file.absolute_path).map_err(|source| Error::Io {
            path: file.absolute_path.clone(),
            source,
        })?;
        members.push(TarMember {
            path: pinned.relative_path.to_owned(),
            contents,
        });
    }
    // LICENSE.md is already in PINNED_SOURCE_FILES; still add ODbL + NOTICE.
    members.push(TarMember {
        path: ODBL_NAME.to_owned(),
        contents: licenses.odbl.clone(),
    });
    members.push(TarMember {
        path: NOTICE_NAME.to_owned(),
        contents: licenses.notice.clone(),
    });
    write_deterministic_ustar(&members)
}

fn read_verified(verified: &VerifiedSourceSet, relative_path: &str) -> Result<String> {
    let file = verified
        .files
        .iter()
        .find(|file| file.relative_path == relative_path)
        .ok_or_else(|| Error::SumoModel(format!("verified set missing {relative_path}")))?;
    fs::read_to_string(&file.absolute_path).map_err(|source| Error::Io {
        path: file.absolute_path.clone(),
        source,
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn workspace_cargo_lock() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")
}

fn resolve_converter_commit(config: &LustConverterConfig) -> Result<String> {
    if let Some(commit) = config.converter_commit.as_ref().filter(|value| !value.is_empty()) {
        return Ok(commit.clone());
    }
    if let Ok(commit) = std::env::var("LANEFLOW_CONVERTER_COMMIT")
        && !commit.is_empty()
    {
        return Ok(commit);
    }
    Err(Error::Config(
        "converter_commit must be set in config or LANEFLOW_CONVERTER_COMMIT".to_owned(),
    ))
}

#[derive(Debug)]
struct TrafficCounts {
    junctions: u64,
    movements: u64,
    maneuver_paths: u64,
    vehicle_profiles: u64,
    signal_controllers: u64,
    signal_groups: u64,
    stop_lines: u64,
    maneuver_gates: u64,
    parking_empty: bool,
}

fn count_traffic_arrays(traffic_bytes: &[u8]) -> Result<TrafficCounts> {
    let value: serde_json::Value =
        serde_json::from_slice(traffic_bytes).map_err(|source| Error::Json {
            document: "TrafficPackage",
            source,
        })?;
    let len = |key: &str| -> Result<u64> {
        value
            .get(key)
            .and_then(|item| item.as_array())
            .map(|items| items.len() as u64)
            .ok_or_else(|| Error::SumoModel(format!("TrafficPackage missing array {key}")))
    };
    let signals = value
        .get("signals")
        .ok_or_else(|| Error::SumoModel("TrafficPackage missing signals".to_owned()))?;
    let signal_len = |key: &str| -> Result<u64> {
        signals
            .get(key)
            .and_then(|item| item.as_array())
            .map(|items| items.len() as u64)
            .ok_or_else(|| Error::SumoModel(format!("TrafficPackage.signals missing array {key}")))
    };
    let parking = value
        .get("parking")
        .ok_or_else(|| Error::SumoModel("TrafficPackage missing parking".to_owned()))?;
    let areas_empty = parking
        .get("areas")
        .and_then(|item| item.as_array())
        .is_some_and(Vec::is_empty);
    let spaces_empty = parking
        .get("spaces")
        .and_then(|item| item.as_array())
        .is_some_and(Vec::is_empty);
    Ok(TrafficCounts {
        junctions: len("junctions")?,
        movements: len("movements")?,
        maneuver_paths: len("maneuverPaths")?,
        vehicle_profiles: len("vehicleProfiles")?,
        signal_controllers: signal_len("controllers")?,
        signal_groups: signal_len("groups")?,
        stop_lines: signal_len("stopLines")?,
        maneuver_gates: signal_len("maneuverGates")?,
        parking_empty: areas_empty && spaces_empty,
    })
}
