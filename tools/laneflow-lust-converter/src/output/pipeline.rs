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
    source::{PINNED_SOURCE_FILES, VerifiedSourceSet, read_verified, verify_source_dir},
    sumo::parse_sumo_network_xml,
};

const NETWORK_LFCA_NAME: &str = "network.lfca";
const ROUTES_NAME: &str = "routes.toml";
const MANIFEST_NAME: &str = "manifest.toml";
const REPORT_NAME: &str = "lust-conversion-report.json";
/// 诊断清单交付物（G1 验收重划；与全网验收测试同一渲染器、同一基线逐字节
/// 一致，文件名与验收落盘 issue253-infeasible-survey.md 保持一致）。
const SURVEY_NAME: &str = "issue253-infeasible-survey.md";
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
    /// fail-fast 路径产出；诊断模式（G1 重划）不交付 network.lfca，为 None。
    pub network_lfca: Option<PathBuf>,
    /// fail-fast 路径产出；诊断模式（#253 L1）不产出 routes.toml，为 None。
    pub routes: Option<PathBuf>,
    pub manifest: PathBuf,
    pub conversion_report: PathBuf,
    pub infeasibility_survey: PathBuf,
    pub source_tar: PathBuf,
    /// 诊断模式不产出 static tar（static bundle 不交付），为 None。
    pub static_tar: Option<PathBuf>,
    pub semantic_provenance: PathBuf,
    pub build_provenance: PathBuf,
}

/// Verify pinned source and emit static bundle + provenance under `output_dir`.
pub fn convert_with_config(
    config: &LustConverterConfig,
    config_toml_bytes: &[u8],
) -> Result<ConvertOutputPaths> {
    // #253 Q7：公开入口校验 config——空 output_dir 会让 join 落进 CWD，M2
    // 的排除产物清除可能误删无关文件（load_config 路径已校验，构造式入口
    // 在此补齐）。
    config.validate()?;
    // #253 Q8：provenance 的 config digest 对**生效**配置求值——重解析
    // bytes 并与传入 config 比对，不一致 fail-closed（调用方须走
    // bytes→config 单一路径构造）。
    let reparsed: LustConverterConfig = toml::from_slice(config_toml_bytes).map_err(Error::Toml)?;
    if reparsed != *config {
        return Err(Error::Config(
            "config_toml_bytes does not match the LustConverterConfig passed to convert".to_owned(),
        ));
    }
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
    let poly_xml = read_verified(verified, "scenario/lust.poly.xml")?;
    let license_md = read_verified(verified, "LICENSE.md")?;

    // G1 验收重划后 converter 的交付物是确定性不可行诊断清单（network.lfca
    // 编译另立 G1）；CLI 的 convert 因此显式走诊断模式（#253 C1）——默认
    // fail-fast 语义不变（TopologyConvertOptions::default 仍 fail-fast，见
    // 全网测试的 fail-fast 断言）。
    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        require_lust_population_count: true,
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    // verify-source 已校验 revision + pinned digest；read_verified 在消费时
    // 重哈希绑定（TOCTOU 闭合）。诊断清单摘要对**消费字节**求值（此处经
    // read_verified 绑定后等于 pinned），verified 声明只能由 crate 内受控
    // 构造器产生（ReportSource 字段私有）。
    let report_source = ReportSource::verified(sha256_digest(net_xml.as_bytes()));
    let static_artifacts = convert_static_from_xml_with_due_and_source(
        &net_xml,
        &tll_xml,
        &vtypes_xml,
        [&due0, &due1, &due2],
        &options,
        report_source,
    )?;
    // 诊断交付物：与全网验收测试同一渲染器（InfeasibilityReport::rendered），
    // 同一基线下逐字节一致。
    let survey = static_artifacts
        .topology
        .infeasibility_report
        .as_ref()
        .expect("diagnostic mode delivers the infeasibility survey")
        .rendered
        .clone();

    // 输出集合按模式自洽（#253 N1；G1 §3.6「落地前 network.lfca 静态 bundle
    // 不交付」）：诊断模式交付 survey + routes/manifest/report + source tar +
    // provenance——不打包零字节 network.lfca、不产出 static tar，manifest /
    // provenance 认证 survey 而非空 LFCA。fail-fast 默认路径产物逻辑不变。
    let diagnostic = static_artifacts.topology.infeasibility_report.is_some();
    let network = parse_sumo_network_xml(&net_xml)?;
    let counts = &static_artifacts.topology.counts;
    let manifest = build_manifest_toml(&static_artifacts, diagnostic)?;

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
        signal_phase_count: counts.signal_phases,
        stop_line_count: counts.stop_lines,
        maneuver_gate_count: counts.maneuver_gates,
        population_record_count: static_artifacts.population_record_count as u64,
        require_lust_population_count: true,
        parking_registry_empty: counts.parking_registry_empty,
        parking_polygon_count: crate::sumo::parse_parking_polygon_count(&poly_xml)?,
        unclaimed_signal_arms: static_artifacts.signal_health.clone(),
        major_minor_green_collapsed: true,
        network_lfca_bytes: (!diagnostic).then(|| static_artifacts.topology.network_lfca.clone()),
        infeasibility_survey_bytes: diagnostic.then(|| survey.clone().into_bytes()),
        routes_toml_bytes: static_artifacts.routes_toml.clone(),
        manifest_bytes: manifest.clone(),
    })?;

    let licenses = LicenseArtifacts {
        license_md: license_md.into_bytes(),
        odbl: embedded_odbl_bytes().to_vec(),
        notice: embedded_notice_bytes().to_vec(),
    };

    let source_tar = build_source_tar(verified, &licenses)?;
    // #253 K1：全部 pinned 文件消费完毕，revision 重校验——检查与消费之间
    // checkout 被切换（即便 pinned 字节保留、digest 全过）也使 provenance
    // 的 revision 声称失真；漂移即 fail-closed。
    crate::source::recheck_source_revision(&config.source_dir)?;
    let static_tar = if diagnostic {
        None
    } else {
        Some(write_deterministic_ustar(&[
            TarMember {
                path: NETWORK_LFCA_NAME.to_owned(),
                contents: static_artifacts.topology.network_lfca.clone(),
            },
            TarMember {
                path: ROUTES_NAME.to_owned(),
                contents: static_artifacts
                    .routes_toml
                    .clone()
                    .expect("fail-fast mode delivers routes.toml"),
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
        ])?)
    };

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
        network_lfca_bytes: (!diagnostic).then(|| static_artifacts.topology.network_lfca.clone()),
        infeasibility_survey_bytes: diagnostic.then(|| survey.clone().into_bytes()),
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
            network_lfca: (!diagnostic)
                .then(|| sha256_digest(&static_artifacts.topology.network_lfca)),
            routes_toml: static_artifacts
                .routes_toml
                .as_ref()
                .map(|bytes| sha256_digest(bytes)),
            manifest_toml: sha256_digest(&manifest),
            conversion_report: sha256_digest(&report),
            source_tar: sha256_digest(&source_tar),
            static_tar: static_tar.as_ref().map(|tar| sha256_digest(tar)),
            infeasibility_survey: diagnostic.then(|| sha256_digest(survey.as_bytes())),
        },
    })?;

    if diagnostic {
        // #253 M2：排除产物不得残留——output_dir 复用时，此前 fail-fast 运行
        // 留下的 network.lfca / lust-static.tar 不在本次 manifest/provenance
        // 认证内，残留的陈旧未认证字节可能被当作本次交付；诊断模式显式删除
        // （不要求空目录，保留增量使用体验；删除失败 fail-closed）。
        remove_excluded_artifacts(&config.output_dir)?;
    }

    fs::create_dir_all(&config.output_dir).map_err(|source| Error::Io {
        path: config.output_dir.clone(),
        source,
    })?;

    let paths = ConvertOutputPaths {
        output_dir: config.output_dir.clone(),
        network_lfca: (!diagnostic).then(|| config.output_dir.join(NETWORK_LFCA_NAME)),
        routes: static_artifacts
            .routes_toml
            .as_ref()
            .map(|_| config.output_dir.join(ROUTES_NAME)),
        manifest: config.output_dir.join(MANIFEST_NAME),
        conversion_report: config.output_dir.join(REPORT_NAME),
        infeasibility_survey: config.output_dir.join(SURVEY_NAME),
        source_tar: config.output_dir.join(SOURCE_TAR_NAME),
        static_tar: static_tar
            .as_ref()
            .map(|_| config.output_dir.join(STATIC_TAR_NAME)),
        semantic_provenance: config.output_dir.join(SEMANTIC_NAME),
        build_provenance: config.output_dir.join(BUILD_NAME),
    };

    if let Some(network_lfca) = &paths.network_lfca {
        write_file(network_lfca, &static_artifacts.topology.network_lfca)?;
    }
    if let (Some(path), Some(routes_toml)) = (&paths.routes, &static_artifacts.routes_toml) {
        write_file(path, routes_toml)?;
    }
    write_file(&paths.manifest, &manifest)?;
    write_file(&paths.conversion_report, &report)?;
    write_file(&paths.infeasibility_survey, survey.as_bytes())?;
    write_file(&paths.source_tar, &source_tar)?;
    if let (Some(path), Some(tar)) = (&paths.static_tar, &static_tar) {
        write_file(path, tar)?;
    }
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
        // read_verified 在消费时重哈希与验证记录比对（TOCTOU 闭合，#253 C2）：
        // 验证后、打包前被换的字节在此 fail-closed，不会静默进入 lust-source.tar。
        let contents = read_verified(verified, pinned.relative_path)?.into_bytes();
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

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn workspace_cargo_lock() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")
}

/// 诊断模式下删除本次不交付的排除产物（#253 M2/L1）：output_dir 复用时残留的
/// network.lfca / lust-static.tar / routes.toml（L1 后诊断模式不产出）不被新
/// manifest/provenance 认证，必须清除，避免陈旧未认证字节被当作本次交付。
/// 不存在视为成功；其他删除错误 fail-closed。
fn remove_excluded_artifacts(output_dir: &Path) -> Result<()> {
    for name in [NETWORK_LFCA_NAME, STATIC_TAR_NAME, ROUTES_NAME] {
        let path = output_dir.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path, source }),
        }
    }
    Ok(())
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

/// manifest 以 size + SHA-256 配对交付物（#253 N1）：诊断模式配对
/// `issue253-infeasible-survey.md` / `routes.toml`（不索引不存在的
/// network.lfca）；fail-fast 路径配对 `network.lfca` / `routes.toml`。
fn build_manifest_toml(
    artifacts: &crate::convert::StaticConversionArtifacts,
    diagnostic: bool,
) -> Result<Vec<u8>> {
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
    if diagnostic {
        insert(
            SURVEY_NAME,
            artifacts
                .topology
                .infeasibility_report
                .as_ref()
                .expect("diagnostic mode delivers the survey")
                .rendered
                .as_bytes(),
        );
    } else {
        insert(NETWORK_LFCA_NAME, &artifacts.topology.network_lfca);
    }
    if let Some(routes_toml) = &artifacts.routes_toml {
        insert(ROUTES_NAME, routes_toml);
    }
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
    use std::path::PathBuf;

    use super::{convert_with_config, remove_excluded_artifacts};
    use crate::Error;

    #[test]
    fn convert_with_config_rejects_invalid_config() {
        // #253 Q7：公开入口先 validate config——空 output_dir 会让 join 落进
        // CWD，M2 的排除产物清除可能误删无关文件。
        let config = crate::config::LustConverterConfig {
            source_dir: PathBuf::from("E:/nonexistent-lust"),
            output_dir: PathBuf::new(),
            converter_commit: None,
            source_bundle_url: None,
            static_bundle_url: None,
        };
        let error = convert_with_config(&config, b"").expect_err("empty output_dir");
        match error {
            Error::Config(message) => assert!(message.contains("output_dir"), "{message}"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn convert_with_config_rejects_mismatched_toml_bytes() {
        // #253 Q8：config_toml_bytes 必须与生效 config 一致——provenance 的
        // config digest 对实际生效的配置求值。
        let config = crate::config::LustConverterConfig {
            source_dir: PathBuf::from("E:/nonexistent-lust"),
            output_dir: PathBuf::from("E:/nonexistent-out"),
            converter_commit: None,
            source_bundle_url: None,
            static_bundle_url: None,
        };
        let mismatched = b"source_dir = 'E:/other'
output_dir = 'E:/nonexistent-out'
";
        let error = convert_with_config(&config, mismatched).expect_err("mismatched config bytes");
        match error {
            Error::Config(message) => {
                assert!(message.contains("does not match"), "{message}")
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn remove_excluded_artifacts_clears_stale_static_outputs() {
        // #253 M2：output_dir 复用时，残留的 network.lfca / lust-static.tar
        // 必须被清除；不存在视为成功。
        let root = std::env::temp_dir().join(format!(
            "laneflow-lust-pipeline-stale-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp");
        std::fs::write(root.join("network.lfca"), b"stale-lfca").expect("write stale lfca");
        std::fs::write(root.join("lust-static.tar"), b"stale-tar").expect("write stale tar");
        std::fs::write(root.join("routes.toml"), b"stale-routes").expect("write stale routes");
        std::fs::write(root.join("manifest.toml"), b"keep-me").expect("write kept artifact");

        remove_excluded_artifacts(&root).expect("stale artifacts removed");

        assert!(!root.join("network.lfca").exists());
        assert!(!root.join("lust-static.tar").exists());
        assert!(
            !root.join("routes.toml").exists(),
            "诊断模式不产 routes.toml"
        );
        assert!(root.join("manifest.toml").exists(), "交付产物不得误删");
        // 幂等：再次调用（产物已不存在）必须成功。
        remove_excluded_artifacts(&root).expect("idempotent removal");
        let _ = std::fs::remove_dir_all(&root);
    }
}
