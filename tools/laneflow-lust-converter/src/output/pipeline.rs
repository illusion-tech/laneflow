//! End-to-end convert packaging: report, licenses, tar, provenance.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Error, Result,
    config::LustConverterConfig,
    convert::{DEFAULT_FIXED_DELTA_MS, TopologyConvertOptions},
    convert_static_from_xml_with_due_and_source,
    output::{
        digest::{hex_sha256, sha256_digest},
        geom::ReportSource,
        model::{ManifestCounts, ManifestFileDigest, ManifestToml},
        provenance::{
            BuildInvocation, BuildProvenanceInput, LicenseArtifacts, RawOutputDigests,
            ReleaseAssetUrls, SemanticConfig, SemanticProvenanceInput, build_build_provenance,
            build_semantic_provenance, embedded_notice_bytes, embedded_odbl_bytes,
        },
        report::{ConversionReportInput, build_conversion_report},
        tar::{TarMember, write_deterministic_ustar},
    },
    source::{PINNED_SOURCE_FILES, VerifiedSourceSet, verify_source_dir},
    sumo::parse_sumo_network_xml,
};

const NETWORK_LFCA_NAME: &str = "network.lfca";
const ROUTES_NAME: &str = "routes.toml";
const MANIFEST_NAME: &str = "manifest.toml";
const REPORT_NAME: &str = "lust-conversion-report.json";
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
    pub network_lfca: PathBuf,
    pub routes: PathBuf,
    pub manifest: PathBuf,
    pub conversion_report: PathBuf,
    pub source_tar: PathBuf,
    pub static_tar: PathBuf,
    pub semantic_provenance: PathBuf,
    pub build_provenance: PathBuf,
}

/// Verify pinned source and emit static bundle + provenance under `output_dir`.
pub fn convert_with_config(
    config: &LustConverterConfig,
    config_toml_bytes: &[u8],
) -> Result<ConvertOutputPaths> {
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
        ..TopologyConvertOptions::default()
    };
    // verify-source 已校验 revision + pinned digest；read_verified 在消费时
    // 重哈希绑定（TOCTOU 闭合）。诊断清单摘要对**消费字节**求值（此处经
    // read_verified 绑定后等于 pinned），据此标注「verify-source 已通过」。
    let net_xml_check = hex_sha256(net_xml.as_bytes());
    let report_source = ReportSource {
        net_digest: Some(format!("sha256:{net_xml_check}")),
        verified: true,
    };
    let static_artifacts = convert_static_from_xml_with_due_and_source(
        &net_xml,
        &tll_xml,
        &vtypes_xml,
        [&due0, &due1, &due2],
        &options,
        report_source,
    )?;

    let network = parse_sumo_network_xml(&net_xml)?;
    let counts = &static_artifacts.topology.counts;
    let manifest = build_manifest_toml(&static_artifacts)?;

    let report = build_conversion_report(&ConversionReportInput {
        external_edge_count: network.external_edge_count() as u64,
        external_lane_count: network.external_lane_count() as u64,
        connection_count: network.connections.len() as u64,
        junction_count: counts.junctions,
        movement_count: counts.movements,
        maneuver_path_count: counts.maneuver_paths,
        route_catalog_count: static_artifacts.route_count as u64,
        vehicle_profile_count: counts.vehicle_profiles,
        signal_controller_count: counts.signal_controllers,
        signal_group_count: counts.signal_groups,
        stop_line_count: counts.stop_lines,
        maneuver_gate_count: counts.maneuver_gates,
        population_record_count: static_artifacts.population_record_count as u64,
        require_lust_population_count: true,
        parking_registry_empty: counts.parking_registry_empty,
        major_minor_green_collapsed: true,
        network_lfca_bytes: static_artifacts.topology.network_lfca.clone(),
        routes_toml_bytes: static_artifacts.routes_toml.clone(),
        manifest_bytes: manifest.clone(),
    })?;

    let licenses = LicenseArtifacts {
        license_md: license_md.into_bytes(),
        odbl: embedded_odbl_bytes().to_vec(),
        notice: embedded_notice_bytes().to_vec(),
    };

    let source_tar = build_source_tar(verified, &licenses)?;
    let static_tar = write_deterministic_ustar(&[
        TarMember {
            path: NETWORK_LFCA_NAME.to_owned(),
            contents: static_artifacts.topology.network_lfca.clone(),
        },
        TarMember {
            path: ROUTES_NAME.to_owned(),
            contents: static_artifacts.routes_toml.clone(),
        },
        TarMember {
            path: MANIFEST_NAME.to_owned(),
            contents: manifest.clone(),
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
        semantic_config: SemanticConfig {
            source_bundle_url: config.source_bundle_url.clone(),
            static_bundle_url: config.static_bundle_url.clone(),
        },
        licenses: licenses.clone(),
        release_urls,
        source_tar: source_tar.clone(),
        static_tar: static_tar.clone(),
        network_lfca_bytes: static_artifacts.topology.network_lfca.clone(),
        routes_toml_bytes: static_artifacts.routes_toml.clone(),
        manifest_bytes: manifest.clone(),
        conversion_report_bytes: report.clone(),
    })?;

    let converter_commit = resolve_converter_commit(config)?;
    let cargo_lock_sha256 =
        hex_sha256(
            &fs::read(workspace_cargo_lock()).map_err(|source| Error::Io {
                path: workspace_cargo_lock(),
                source,
            })?,
        );
    let build = build_build_provenance(&BuildProvenanceInput {
        converter_commit,
        rust_version: "1.98.0",
        cargo_lock_sha256,
        config_digest: sha256_digest(config_toml_bytes),
        semantic_provenance_digest: sha256_digest(&semantic),
        invocation: BuildInvocation {
            command: "convert",
            require_lust_location_anchors: true,
            require_lust_population_count: true,
        },
        raw_output_digests: RawOutputDigests {
            network_lfca: sha256_digest(&static_artifacts.topology.network_lfca),
            routes_toml: sha256_digest(&static_artifacts.routes_toml),
            manifest_toml: sha256_digest(&manifest),
            conversion_report: sha256_digest(&report),
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
        network_lfca: config.output_dir.join(NETWORK_LFCA_NAME),
        routes: config.output_dir.join(ROUTES_NAME),
        manifest: config.output_dir.join(MANIFEST_NAME),
        conversion_report: config.output_dir.join(REPORT_NAME),
        source_tar: config.output_dir.join(SOURCE_TAR_NAME),
        static_tar: config.output_dir.join(STATIC_TAR_NAME),
        semantic_provenance: config.output_dir.join(SEMANTIC_NAME),
        build_provenance: config.output_dir.join(BUILD_NAME),
    };

    write_file(&paths.network_lfca, &static_artifacts.topology.network_lfca)?;
    write_file(&paths.routes, &static_artifacts.routes_toml)?;
    write_file(&paths.manifest, &manifest)?;
    write_file(&paths.conversion_report, &report)?;
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
            .ok_or_else(|| {
                Error::SumoModel(format!(
                    "verified set missing pinned file {}",
                    pinned.relative_path
                ))
            })?;
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

/// 读入 verify-source 快照文件并在**消费时**重算 SHA-256 与验证记录比对：
/// 验证与消费之间字节被换（TOCTOU）即 fail-closed，消费字节由此与
/// pinned 校验绑定（#253 R2）。
fn read_verified(verified: &VerifiedSourceSet, relative_path: &str) -> Result<String> {
    let file = verified
        .files
        .iter()
        .find(|file| file.relative_path == relative_path)
        .ok_or_else(|| Error::SumoModel(format!("verified set missing {relative_path}")))?;
    let bytes = fs::read(&file.absolute_path).map_err(|source| Error::Io {
        path: file.absolute_path.clone(),
        source,
    })?;
    let actual = hex_sha256(&bytes);
    if actual != file.sha256_hex {
        return Err(Error::SourceChangedAfterVerification {
            relative_path: file.relative_path,
            expected: file.sha256_hex.clone(),
            actual,
        });
    }
    String::from_utf8(bytes)
        .map_err(|_| Error::SumoModel(format!("verified {relative_path} is not UTF-8")))
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
    if let Some(commit) = config
        .converter_commit
        .as_ref()
        .filter(|value| !value.is_empty())
    {
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

fn build_manifest_toml(artifacts: &crate::convert::StaticConversionArtifacts) -> Result<Vec<u8>> {
    let counts = &artifacts.topology.counts;
    let mut files = BTreeMap::new();
    let mut insert = |name: &str, bytes: &[u8]| {
        files.insert(
            name.to_owned(),
            ManifestFileDigest {
                bytes: u64::try_from(bytes.len()).expect("artifact size fits u64"),
                sha256: sha256_digest(bytes),
            },
        );
    };
    insert(NETWORK_LFCA_NAME, &artifacts.topology.network_lfca);
    insert(ROUTES_NAME, &artifacts.routes_toml);
    let manifest = ManifestToml {
        manifest_version: 1,
        generator: "laneflow-lust-converter",
        fixed_step_ms: DEFAULT_FIXED_DELTA_MS,
        counts: ManifestCounts {
            lane_edges: counts.lane_edges,
            junctions: counts.junctions,
            movements: counts.movements,
            maneuver_paths: counts.maneuver_paths,
            vehicle_profiles: counts.vehicle_profiles,
            signal_controllers: counts.signal_controllers,
            signal_groups: counts.signal_groups,
            stop_lines: counts.stop_lines,
            maneuver_gates: counts.maneuver_gates,
            routes: artifacts.route_count as u64,
            population_records: artifacts.population_record_count as u64,
            parking_registry_empty: counts.parking_registry_empty,
        },
        files,
    };
    let text = toml::to_string_pretty(&manifest).map_err(|source| Error::TomlSerialize {
        document: "manifest.toml",
        source,
    })?;
    Ok(text.into_bytes())
}
#[cfg(test)]
mod tests {
    use super::{hex_sha256, read_verified};
    use crate::{
        Error,
        source::{VerifiedSourceFile, VerifiedSourceSet},
    };

    fn snapshot_with(
        root: &std::path::Path,
        relative_path: &'static str,
        bytes: &[u8],
    ) -> VerifiedSourceSet {
        let absolute_path = root.join(relative_path);
        std::fs::create_dir_all(absolute_path.parent().expect("parent dir"))
            .expect("create parent");
        std::fs::write(&absolute_path, bytes).expect("write snapshot file");
        VerifiedSourceSet {
            source_dir: root.to_path_buf(),
            files: vec![VerifiedSourceFile {
                relative_path,
                absolute_path,
                bytes: bytes.len() as u64,
                sha256_hex: hex_sha256(bytes),
            }],
        }
    }

    #[test]
    fn read_verified_accepts_intact_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-pipeline-intact-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        let text =
            read_verified(&snapshot, "scenario/lust.net.xml").expect("intact snapshot reads");
        assert_eq!(text, "<net/>");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_verified_rejects_bytes_changed_after_verification() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-pipeline-toctou-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        // 验证与消费之间文件被换（TOCTOU）：同尺寸换内容必须 fail-closed。
        std::fs::write(&snapshot.files[0].absolute_path, b"<NET/>").expect("swap bytes");
        let error = read_verified(&snapshot, "scenario/lust.net.xml")
            .expect_err("changed bytes must fail closed");
        match error {
            Error::SourceChangedAfterVerification {
                relative_path,
                expected,
                actual,
            } => {
                assert_eq!(relative_path, "scenario/lust.net.xml");
                assert_eq!(expected, hex_sha256(b"<net/>"));
                assert_eq!(actual, hex_sha256(b"<NET/>"));
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_verified_reports_snapshot_missing_file() {
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-pipeline-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        let snapshot = snapshot_with(&root, "scenario/lust.net.xml", b"<net/>");
        let error = read_verified(&snapshot, "scenario/tll.static.xml")
            .expect_err("unverified file must not be consumed");
        match error {
            Error::SumoModel(message) => {
                assert!(message.contains("scenario/tll.static.xml"), "{message}");
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
