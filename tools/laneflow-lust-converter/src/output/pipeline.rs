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

    let source_tar = build_source_tar(verified)?;
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
            source_tar: sha256_digest(source_tar.bytes()),
            static_tar: static_tar.as_ref().map(|tar| sha256_digest(tar)),
            infeasibility_survey: diagnostic.then(|| sha256_digest(survey.as_bytes())),
        },
    })?;

    // #253 T2：stage-then-publish——全部产物先写 output_dir 旁的 staging
    // 目录，全部写成功后再替换进 output_dir。转换中途失败时交付集合不被
    // 污染（旧文件保持原样）；排除产物清除挪到 publish 阶段先执行。
    let staging = staging_dir(&config.output_dir);
    fs::create_dir_all(&staging).map_err(|source| Error::Io {
        path: staging.clone(),
        source,
    })?;

    let stage_outputs = || -> Result<Vec<(&'static str, &'static str)>> {
        let staged: Vec<(&'static str, Option<&[u8]>)> = vec![
            (
                NETWORK_LFCA_NAME,
                (!diagnostic).then_some(static_artifacts.topology.network_lfca.as_slice()),
            ),
            (ROUTES_NAME, static_artifacts.routes_toml.as_deref()),
            (MANIFEST_NAME, Some(manifest.as_slice())),
            (REPORT_NAME, Some(report.as_slice())),
            (SURVEY_NAME, Some(survey.as_bytes())),
            (SOURCE_TAR_NAME, Some(source_tar.bytes())),
            (STATIC_TAR_NAME, static_tar.as_deref()),
            (SEMANTIC_NAME, Some(semantic.as_slice())),
            (BUILD_NAME, Some(build.as_slice())),
            (LICENSE_NAME, Some(licenses.license_md.as_slice())),
            (ODBL_NAME, Some(licenses.odbl.as_slice())),
            (NOTICE_NAME, Some(licenses.notice.as_slice())),
        ];
        let mut written = Vec::with_capacity(staged.len());
        for (name, bytes) in staged {
            if let Some(bytes) = bytes {
                write_file(&staging.join(name), bytes)?;
                written.push((name, name));
            }
        }
        Ok(written)
    };
    let staged = match stage_outputs() {
        Ok(staged) => staged,
        Err(error) => {
            // 失败：output_dir 原样保留，清理 staging。
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

    let publish = publish_outputs(&staging, &config.output_dir, &staged);
    let _ = fs::remove_dir_all(&staging);
    publish?;

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
    Ok(paths)
}

/// staging 目录：output_dir 的兄弟目录 `.staging-<pid>-<name>`（同卷，
/// 保证 publish 的 rename 可用）。
fn staging_dir(output_dir: &Path) -> PathBuf {
    let name = output_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_owned());
    let parent = output_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    parent.join(format!(".staging-{}-{name}", std::process::id()))
}

/// publish（#253 U2 事务式）：先把 output_dir 现有交付文件与排除产物移入
/// 备份区（staging 旁的 `.backup-<pid>-<name>`，同卷），再逐个移入新文件；
/// 任一步失败从备份区恢复旧集合并 fail-closed，成功后清备份（排除产物随
/// 备份丢弃——M2 语义；诊断模式 on success 不恢复它们，on failure 恢复）。
/// Windows：rename 不覆盖、目录不可整体替换，全部逐文件进行。
fn publish_outputs(
    staging: &Path,
    output_dir: &Path,
    staged: &[(&'static str, &'static str)],
) -> Result<()> {
    fs::create_dir_all(output_dir).map_err(|source| Error::Io {
        path: output_dir.to_path_buf(),
        source,
    })?;
    let backup = backup_dir(output_dir);
    fs::create_dir_all(&backup).map_err(|source| Error::Io {
        path: backup.clone(),
        source,
    })?;
    let result = swap_outputs(staging, output_dir, &backup, staged);
    if result.is_err() {
        restore_backup(output_dir, &backup, staged);
    }
    let _ = fs::remove_dir_all(&backup);
    result
}

fn backup_dir(output_dir: &Path) -> PathBuf {
    let name = output_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_owned());
    let parent = output_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    parent.join(format!(".backup-{}-{name}", std::process::id()))
}

fn swap_outputs(
    staging: &Path,
    output_dir: &Path,
    backup: &Path,
    staged: &[(&'static str, &'static str)],
) -> Result<()> {
    // 备份现存：交付名 ∪ 排除名。
    let mut names: Vec<&'static str> = staged.iter().map(|(name, _)| *name).collect();
    for excluded in [NETWORK_LFCA_NAME, STATIC_TAR_NAME, ROUTES_NAME] {
        if !names.contains(&excluded) {
            names.push(excluded);
        }
    }
    for name in &names {
        let dest = output_dir.join(name);
        if dest.symlink_metadata().is_ok() {
            fs::rename(&dest, backup.join(name)).map_err(|source| Error::Io {
                path: dest.clone(),
                source,
            })?;
        }
    }
    // 移入新文件。
    for (name, _) in staged {
        let from = staging.join(name);
        let to = output_dir.join(name);
        fs::rename(&from, &to).map_err(|source| Error::Io {
            path: from.clone(),
            source,
        })?;
    }
    Ok(())
}

/// 失败恢复：备份区有的文件逐个移回 output_dir（新文件若已落位先删）。
/// 尽力而为——恢复本身的残余失败由清理与错误信息暴露。
fn restore_backup(output_dir: &Path, backup: &Path, staged: &[(&'static str, &'static str)]) {
    let mut names: Vec<&'static str> = staged.iter().map(|(name, _)| *name).collect();
    for excluded in [NETWORK_LFCA_NAME, STATIC_TAR_NAME, ROUTES_NAME] {
        if !names.contains(&excluded) {
            names.push(excluded);
        }
    }
    for name in names {
        let from = backup.join(name);
        if from.symlink_metadata().is_err() {
            continue;
        }
        let to = output_dir.join(name);
        let _ = fs::remove_file(&to);
        let _ = fs::rename(&from, &to);
    }
}

/// 由已验证 source 集构建确定性 source tar（#253 T3：返回类型级绑定的
/// `VerifiedSourceTar`——provenance 的 pinned source 断言只接受验证路径产物）。
pub(crate) fn build_source_tar(
    verified: &VerifiedSourceSet,
) -> Result<crate::output::provenance::VerifiedSourceTar> {
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
        contents: embedded_odbl_bytes().to_vec(),
    });
    members.push(TarMember {
        path: NOTICE_NAME.to_owned(),
        contents: embedded_notice_bytes().to_vec(),
    });
    let bytes = write_deterministic_ustar(&members)?;
    Ok(crate::output::provenance::VerifiedSourceTar { bytes })
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

    use super::{MANIFEST_NAME, SURVEY_NAME, convert_with_config, publish_outputs};
    use crate::Error;

    #[test]
    fn publish_outputs_replaces_files_and_clears_excluded() {
        // #253 T2：publish 先清排除产物，再逐文件替换——staging 内容进
        // output_dir，stale 排除产物被清除。
        let root = std::env::temp_dir().join(format!("lust-publish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        std::fs::write(output.join("network.lfca"), b"stale").expect("stale");
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");
        std::fs::write(staging.join("issue253-infeasible-survey.md"), b"survey").expect("sv");

        publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (SURVEY_NAME, SURVEY_NAME)],
        )
        .expect("publish");

        assert_eq!(
            std::fs::read(output.join("manifest.toml")).expect("read"),
            b"new-manifest"
        );
        assert!(output.join("issue253-infeasible-survey.md").exists());
        assert!(!output.join("network.lfca").exists(), "排除产物已清除");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_failure_restores_original_set() {
        // #253 U2 事务式：中途失败从备份区恢复旧集合——manifest 回到旧内容，
        // survey 目录未被新文件替换，排除产物（network.lfca）原样恢复。
        let root = std::env::temp_dir().join(format!("lust-publish-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        std::fs::write(output.join("network.lfca"), b"stale-lfca").expect("stale");
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");
        // 目标位置放一个**目录**占用 survey 文件名——移入失败，触发恢复。
        std::fs::create_dir(output.join(SURVEY_NAME)).expect("dir squat");

        let result = publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (SURVEY_NAME, SURVEY_NAME)],
        );
        assert!(result.is_err(), "replace 失败必须 fail-closed");
        assert_eq!(
            std::fs::read(output.join("manifest.toml")).expect("read"),
            b"old-manifest",
            "已移入的新文件必须被恢复为旧内容"
        );
        assert_eq!(
            std::fs::read(output.join("network.lfca")).expect("read"),
            b"stale-lfca",
            "排除产物在失败路径也必须恢复"
        );
        assert!(output.join(SURVEY_NAME).is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

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
}
