//! End-to-end convert packaging: report, licenses, tar, provenance.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Error, Result,
    config::LustConverterConfig,
    convert::{
        DEFAULT_FIXED_DELTA_MS, TopologyConvertOptions,
        topology::convert_static_from_xml_with_due_and_source,
    },
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
/// 备份阶段完成标记（写在备份目录内）：装入开始前写入，中断恢复据此
/// 区分崩溃形态（见 `recover_interrupted_publish`）。
const BACKUP_COMPLETE_MARKER: &str = ".backup-complete";

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

/// Verify pinned source and emit the deterministic artifact set + provenance
/// under `output_dir`（诊断模式交付 survey/source tar 等，不产出 static
/// bundle；`ConvertOutputPaths` 的 Option 字段按模式为 None）。
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
    // 中断恢复先于一切可失败操作：source 验证、转换、provenance、staging
    // 任一失败都到不了 publish——上次运行在 swap 中途被杀时，在此恢复
    // output_dir 的旧交付集。公开入口固定诊断模式（convert_verified 的
    // options 硬编码 emit_infeasibility_report），交付名列表同源。
    let deliverables: Vec<(&'static str, &'static str)> = deliverable_names(true)
        .into_iter()
        .map(|name| (name, name))
        .collect();
    // 持锁前把 output_dir 锚定到稳定绝对路径：入口 cwd 是绝对化回退的
    // 基准，避免相对 output_dir 被进程级 set_current_dir 漂移改指。
    let entry_cwd = std::env::current_dir().map_err(|source| Error::Validation {
        stage: "anchor",
        message: format!("could not determine the process current directory: {source}"),
    })?;
    let anchor = output_anchor(&config.output_dir, &entry_cwd);
    // 排他锁覆盖 恢复→发布 全程：两进程并发同 output_dir 时，后者会把
    // 前者的在途备份误判为中断事务并恢复，造成新旧产物混杂。进程退出
    // 由 OS 自动放锁，崩溃不留死锁；锁文件是 output_dir 的兄弟点文件，
    // 不进入交付集。
    let _output_lock = acquire_output_lock(&anchor)?;
    recover_interrupted_publish(&anchor, &deliverables)?;
    // 崩溃遗留的 staging 目录随恢复一并清理——内容是纯新产物副本，
    // 不是任何事物的唯一副本；删除失败不阻塞本次转换。
    remove_stale_staging(&anchor);
    let verified = verify_source_dir(&config.source_dir)?;
    convert_verified(config, config_toml_bytes, &verified, &anchor, &entry_cwd)
}

fn convert_verified(
    config: &LustConverterConfig,
    config_toml_bytes: &[u8],
    verified: &VerifiedSourceSet,
    anchor: &Path,
    entry_cwd: &Path,
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

    // 打包前释放已消费完毕的源输入缓冲（合计约 143 MB）：net/tll/vtypes/
    // DUE/poly 此后不再使用。不释放则与 build_source_tar 重读出的
    // TarMember 缓冲及 tar 输出并存，峰值约三份源体量，受限 runner 有
    // OOM 风险。
    drop(net_xml);
    drop(tll_xml);
    drop(vtypes_xml);
    drop(due0);
    drop(due1);
    drop(due2);
    drop(poly_xml);
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
        semantic_config: semantic_config(config, diagnostic),
        licenses: licenses.clone(),
        release_urls,
        source_tar: &source_tar,
        static_tar: static_tar.as_deref(),
        network_lfca_bytes: (!diagnostic)
            .then_some(static_artifacts.topology.network_lfca.as_slice()),
        infeasibility_survey_bytes: diagnostic.then_some(survey.as_bytes()),
        routes_toml_bytes: static_artifacts.routes_toml.as_deref(),
        manifest_bytes: manifest.as_slice(),
        conversion_report_bytes: report.as_slice(),
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
    // staging 挂在入口持锁锚点旁（不重解析 output_dir——见 publish_outputs
    // 的 anchor 契约；符号链接跨盘时词法 parent 会让 rename 跨设备失败）。
    let staging = staging_dir(anchor);
    fs::create_dir_all(&staging).map_err(|source| Error::Io {
        path: staging.clone(),
        source,
    })?;

    let stage_outputs = || -> Result<Vec<(&'static str, &'static str)>> {
        let bytes_by_name: [(&'static str, Option<&[u8]>); 12] = [
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
        let mut written = Vec::with_capacity(bytes_by_name.len());
        // 交付名列表的唯一事实源是 deliverable_names——中断恢复的早期
        // 调用与 staged 名单由此保持一致。
        for name in deliverable_names(diagnostic) {
            let bytes = bytes_by_name
                .iter()
                .find(|(entry, _)| *entry == name)
                .and_then(|(_, bytes)| *bytes)
                .expect("deliverable must have staged bytes");
            write_file(&staging.join(name), bytes)?;
            written.push((name, name));
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

    let publish = publish_outputs(&staging, anchor, &staged);
    let _ = fs::remove_dir_all(&staging);
    publish?;

    // 返回路径以词法 output_dir 拼接（保持调用方视角），但交付实际落在
    // 入口持锁锚点上——output_dir 符号链接若在转换中途被改指，词法路径会
    // 指向未写入的新目标。返回前校验别名仍解析到同一锚点，漂移即
    // fail-closed 并指明实际落位。
    let current_anchor = output_anchor(&config.output_dir, entry_cwd);
    if current_anchor != *anchor {
        return Err(Error::Validation {
            stage: "publish",
            message: format!(
                "delivery published at locked anchor {}, but output_dir {} now resolves to {} — reconcile the symlink before consuming the outputs",
                anchor.display(),
                config.output_dir.display(),
                current_anchor.display()
            ),
        });
    }

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

/// 语义配置子集（configDigest 输入）：诊断模式不交付 static bundle
/// （static_tar 为 None），未发射资产的 URL 不得进入语义摘要——否则仅
/// 改动该闲置 URL 就会改变逐字节相同诊断交付的 configDigest（跨构建
/// 比对身份）。
fn semantic_config(config: &LustConverterConfig, diagnostic: bool) -> SemanticConfig {
    SemanticConfig {
        source_bundle_url: config.source_bundle_url.clone(),
        static_bundle_url: if diagnostic {
            None
        } else {
            config.static_bundle_url.clone()
        },
    }
}

/// output_dir 的排他转换锁（兄弟文件 `.lock-<name>`，同卷）：覆盖
/// 恢复→发布 全程，防止并发 convert 把对方的在途备份误判为中断事务。
/// 锁本体由 OS 管理（进程退出即放），崩溃残留的锁文件不影响下次运行。
fn acquire_output_lock(anchor: &Path) -> Result<fs::File> {
    let name = anchor
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_owned());
    let parent = anchor.parent().map(Path::to_path_buf).unwrap_or_default();
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(&parent).map_err(|source| Error::Io {
            path: parent.clone(),
            source,
        })?;
    }
    let lock_path = parent.join(format!(".lock-{name}"));
    let file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| Error::Io {
            path: lock_path.clone(),
            source,
        })?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::Error(source) => Error::Io {
            path: lock_path.clone(),
            source,
        },
        std::fs::TryLockError::WouldBlock => Error::Validation {
            stage: "lock",
            message: format!(
                "another convert process holds the output lock for {} (lock file: {})",
                anchor.display(),
                lock_path.display()
            ),
        },
    })?;
    Ok(file)
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

/// staging/backup 的锚定路径：output_dir 若是（跨盘）符号链接，词法
/// parent 会把 staging/backup 放在链接侧，publish 的 rename 跨设备失败
/// （EXDEV）。存在则解析到真实路径；尚不存在则解析 parent 再拼回名字；
/// 都失败时相对 `base`（入口 cwd）绝对化——锚点必须全程绝对且稳定：
/// 相对回退会被进程级 set_current_dir 漂移改指，锁在旧 cwd 下持有而
/// staging/publish 在新 cwd 下解析（可能发布进错误目录并与那边的
/// converter 竞争）。
fn output_anchor(output_dir: &Path, base: &Path) -> PathBuf {
    if let Ok(resolved) = fs::canonicalize(output_dir) {
        return resolved;
    }
    if let Some(parent) = output_dir.parent()
        && let Ok(resolved_parent) = fs::canonicalize(parent)
        && let Some(name) = output_dir.file_name()
    {
        return resolved_parent.join(name);
    }
    base.join(output_dir)
}

/// 交付文件名（staged 名单的唯一事实源；诊断模式不交付 network.lfca /
/// routes.toml / static tar）。中断恢复的早期调用与 stage_outputs 共用。
fn deliverable_names(diagnostic: bool) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(12);
    if !diagnostic {
        names.push(NETWORK_LFCA_NAME);
        names.push(ROUTES_NAME);
    }
    names.extend([MANIFEST_NAME, REPORT_NAME, SURVEY_NAME, SOURCE_TAR_NAME]);
    if !diagnostic {
        names.push(STATIC_TAR_NAME);
    }
    names.extend([
        SEMANTIC_NAME,
        BUILD_NAME,
        LICENSE_NAME,
        ODBL_NAME,
        NOTICE_NAME,
    ]);
    names
}

/// publish（#253 U2 事务式）：先把 output_dir 现有交付文件与排除产物移入
/// 备份区（staging 旁的 `.backup-<pid>-<name>`，同卷），再逐个移入新文件；
/// 任一步失败从备份区恢复旧集合并 fail-closed，成功后清备份（排除产物随
/// 备份丢弃——M2 语义；诊断模式 on success 不恢复它们，on failure 恢复）。
/// Windows：rename 不覆盖、目录不可整体替换，全部逐文件进行。
///
/// 中断恢复：进程在 swap 中途被杀时残留的 `.backup-*` 目录由
/// `recover_interrupted_publish` 在本函数开头处理。回滚不完整时保留备份
/// 目录（旧交付集的唯一副本可能在其中）并在错误信息中指明。成功后的备份
/// 清理失败同样报错（残留带标记备份会让下次运行 fail-closed 报歧义，不在
/// 成功路径静默埋雷）。
///
/// `anchor` 必须是入口已持锁的解析锚点（`output_anchor` 在持锁前解析
/// 一次）：转换全程不再对可变的 output_dir 重解析——否则 symlink 中途
/// 改指会让 staging/publish 落到新目标，而排他锁仍护旧目标。
fn publish_outputs(
    staging: &Path,
    anchor: &Path,
    staged: &[(&'static str, &'static str)],
) -> Result<()> {
    fs::create_dir_all(anchor).map_err(|source| Error::Io {
        path: anchor.to_path_buf(),
        source,
    })?;
    recover_interrupted_publish(anchor, staged)?;
    let backup = backup_dir(anchor);
    fs::create_dir_all(&backup).map_err(|source| Error::Io {
        path: backup.clone(),
        source,
    })?;
    let mut installed = Vec::new();
    let publish_error = match swap_outputs(staging, anchor, &backup, staged, &mut installed) {
        Ok(()) => {
            // 成功清理：先逐个删除备份内受管文件，再删标记与目录——任一步
            // 删不动（典型如 Windows 临时文件锁）都保留备份并立即报错指明
            // 位置：残留带标记备份会让下次运行 fail-closed 报歧义，不在
            // 成功路径静默埋雷。
            let mut cleanup_ok = true;
            for name in managed_names(staged) {
                let path = backup.join(name);
                if path.symlink_metadata().is_ok() && fs::remove_file(&path).is_err() {
                    cleanup_ok = false;
                }
            }
            if cleanup_ok {
                cleanup_ok = fs::remove_file(backup.join(BACKUP_COMPLETE_MARKER)).is_ok()
                    && fs::remove_dir_all(&backup).is_ok();
            }
            if cleanup_ok {
                return Ok(());
            }
            return Err(Error::Validation {
                stage: "cleanup",
                message: format!(
                    "delivery published, but backup cleanup failed; backup preserved at {} — inspect and remove it manually before the next run",
                    backup.display()
                ),
            });
        }
        Err(error) => error,
    };
    match restore_backup(anchor, &backup, staged, &installed) {
        Ok(()) => {
            // 恢复完成后才允许清备份：回滚不完整时备份是旧交付集的唯一副本。
            let _ = fs::remove_file(backup.join(BACKUP_COMPLETE_MARKER));
            let _ = fs::remove_dir_all(&backup);
            Err(publish_error)
        }
        Err(failures) => Err(Error::Validation {
            stage: "publish",
            message: format!(
                "publication failed: {publish_error}; rollback incomplete ({failures}); backup preserved at {} (next run retries recovery)",
                backup.display()
            ),
        }),
    }
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
    installed: &mut Vec<&'static str>,
) -> Result<()> {
    // 备份现存：交付名 ∪ 排除名。
    for name in managed_names(staged) {
        let dest = output_dir.join(name);
        if dest.symlink_metadata().is_ok() {
            fs::rename(&dest, backup.join(name)).map_err(|source| Error::Io {
                path: dest.clone(),
                source,
            })?;
        }
    }
    // 备份阶段完成标记——装入开始前写入；崩溃恢复据此区分中断形态。
    fs::write(backup.join(BACKUP_COMPLETE_MARKER), b"").map_err(|source| Error::Io {
        path: backup.join(BACKUP_COMPLETE_MARKER),
        source,
    })?;
    // 移入新文件，记录实际装入成功的名字——回滚只删这些，不动未触及的旧文件。
    for (name, _) in staged {
        let from = staging.join(name);
        let to = output_dir.join(name);
        fs::rename(&from, &to).map_err(|source| Error::Io {
            path: from.clone(),
            source,
        })?;
        installed.push(name);
    }
    Ok(())
}

/// 受管文件名：交付名 ∪ 排除产物名（M2）。
fn managed_names(staged: &[(&'static str, &'static str)]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = staged.iter().map(|(name, _)| *name).collect();
    for excluded in [NETWORK_LFCA_NAME, STATIC_TAR_NAME, ROUTES_NAME] {
        if !names.contains(&excluded) {
            names.push(excluded);
        }
    }
    names
}

/// 中断恢复：上次运行在 swap 中途被杀会留下 `.backup-<pid>-<name>` 残留。
/// 不变量：备份阶段逐文件先于一切装入，备份完成后才写标记——据此区分
/// 两种崩溃形态并确定性恢复。恢复失败或存在多个残留时 fail-closed
/// （备份保留，人工处理）。
fn recover_interrupted_publish(
    output_dir: &Path,
    staged: &[(&'static str, &'static str)],
) -> Result<()> {
    let stale = find_stale_backups(output_dir)?;
    if stale.is_empty() {
        return Ok(());
    }
    // 恢复目标目录可能尚不存在（上次运行在创建 output_dir 前就被杀）。
    fs::create_dir_all(output_dir).map_err(|source| Error::Io {
        path: output_dir.to_path_buf(),
        source,
    })?;
    if stale.len() > 1 {
        return Err(Error::Validation {
            stage: "publish",
            message: format!(
                "multiple stale backup directories next to {}: {} — resolve manually before re-running",
                output_dir.display(),
                stale
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    let backup = &stale[0];
    let marker = backup.join(BACKUP_COMPLETE_MARKER);
    if marker.symlink_metadata().is_ok() {
        // 中断于装入阶段：备份是全量旧集。但 output_dir 已含全部 staged
        // 文件时无法区分「装入末尾崩溃」与「成功后清理失败」——fail-closed
        // 交人工，绝不把可能成功的交付静默回滚。
        let output_complete = staged
            .iter()
            .all(|(name, _)| output_dir.join(name).symlink_metadata().is_ok());
        if output_complete {
            return Err(Error::Validation {
                stage: "publish",
                message: format!(
                    "stale backup {} coexists with a complete output set; cannot tell an interrupted publish from a failed cleanup — inspect and remove the backup manually",
                    backup.display()
                ),
            });
        }
        // 装入阶段中断时，无备份对应物的 staged 名字必是新装入文件。
        let installed: Vec<&'static str> = staged
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !backup.join(name).symlink_metadata().is_ok())
            .collect();
        restore_backup(output_dir, backup, staged, &installed).map_err(|failures| {
            Error::Validation {
                stage: "publish",
                message: format!(
                    "recovery from interrupted publish incomplete ({failures}); backup preserved at {}",
                    backup.display()
                ),
            }
        })?;
    } else {
        // 中断于备份阶段：装入从未发生，只恢复备份内容，output_dir 里
        // 尚未备份的旧文件原样保留。
        restore_backup(output_dir, backup, staged, &[]).map_err(|failures| Error::Validation {
            stage: "publish",
            message: format!(
                "recovery from interrupted backup incomplete ({failures}); backup preserved at {}",
                backup.display()
            ),
        })?;
    }
    let _ = fs::remove_file(&marker);
    fs::remove_dir_all(backup).map_err(|source| Error::Io {
        path: backup.clone(),
        source,
    })?;
    Ok(())
}

/// 找 output_dir 旁的残留备份目录：`.backup-<纯数字 pid>-<name>`。
fn find_stale_backups(output_dir: &Path) -> Result<Vec<PathBuf>> {
    let name = output_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_owned());
    let parent = output_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let suffix = format!("-{name}");
    let mut found = Vec::new();
    let entries = match fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(source) => {
            return Err(Error::Io {
                path: parent.clone(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: parent.clone(),
            source,
        })?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if stale_dir_name_matches(&file_name, ".backup-", &suffix) {
            found.push(entry.path());
        }
    }
    Ok(found)
}

/// 匹配 `.<前缀>-<纯数字 pid>-<name>` 形态的残留目录名。
fn stale_dir_name_matches(file_name: &str, prefix: &str, suffix: &str) -> bool {
    file_name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
}

/// 清理残留 staging 目录（`.staging-<pid>-<name>`）：进程在 staging 创建
/// 后、publish 完成前被杀就会留下——内容是纯新产物副本（旧交付集的唯一
/// 副本在备份区，不在此处），删除安全。尽力而为：删除失败留待下次运行
/// 重试，不阻塞本次转换。
fn remove_stale_staging(anchor: &Path) {
    let name = anchor
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_owned());
    let parent = anchor.parent().map(Path::to_path_buf).unwrap_or_default();
    let suffix = format!("-{name}");
    let Ok(entries) = fs::read_dir(&parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if stale_dir_name_matches(&file_name, ".staging-", &suffix) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// 失败恢复：备份区有的文件逐个**复制**回 output_dir（新文件若已落位先删）；
/// installed 名单（实际装入成功）里无备份对应物的文件删除。恢复用 copy
/// 而非 rename：备份条目在恢复提交（调用方删标记与目录）前不被消耗——
/// 恢复中途被杀/部分失败时备份仍完整，重试幂等，「备份缺条目 ⟺ 运行前
/// 不存在」的 installed 判据因此恒成立（move 语义下已归还的旧文件会被
/// 下次恢复误判为新装入而删除）。任一文件恢复失败即收集进 Err——调用方
/// 据此保留备份目录（旧交付集唯一副本在其中），不再无条件清理。
fn restore_backup(
    output_dir: &Path,
    backup: &Path,
    staged: &[(&'static str, &'static str)],
    installed: &[&'static str],
) -> std::result::Result<(), String> {
    let mut failures: Vec<String> = Vec::new();
    for name in managed_names(staged) {
        let from = backup.join(name);
        let to = output_dir.join(name);
        if from.symlink_metadata().is_ok() {
            // 旧文件：新文件若已落位先删，再从备份恢复。
            if let Err(source) = fs::remove_file(&to)
                && source.kind() != std::io::ErrorKind::NotFound
            {
                failures.push(format!("remove {}: {source}", to.display()));
                continue;
            }
            if let Err(source) = fs::copy(&from, &to) {
                failures.push(format!("restore {}: {source}", to.display()));
            }
        } else if installed.contains(&name) && to.symlink_metadata().is_ok() {
            // #253 V1：本次新装入、运行前不存在的文件——失败路径必须删除，
            // 保证错误返回时 output_dir 完全回到运行前状态。判据是实际装入
            // 名单而非 staged 名单：备份阶段中途失败时，尚未轮到的文件无
            // 备份条目却还带着运行前的旧文件，误删会丢掉旧交付集。
            if let Err(source) = fs::remove_file(&to) {
                failures.push(format!("remove installed {}: {source}", to.display()));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
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
    artifacts: &crate::convert::topology::StaticConversionArtifacts,
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

    use super::{
        BACKUP_COMPLETE_MARKER, MANIFEST_NAME, REPORT_NAME, SURVEY_NAME, acquire_output_lock,
        backup_dir, convert_with_config, publish_outputs, remove_stale_staging, restore_backup,
        semantic_config, swap_outputs,
    };
    #[cfg(unix)]
    use super::{output_anchor, staging_dir};
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
    fn publish_outputs_failure_removes_newly_installed_files() {
        // #253 V1：失败路径须删除「本次新装入、运行前不存在」的文件——
        // output_dir 完全回到运行前状态。
        let root =
            std::env::temp_dir().join(format!("lust-publish-newfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        // report.json 是**新交付**（运行前不存在）；staging 缺 survey 文件——
        // 失败确定发生在移入阶段（manifest 与 report.json 已装入），不依赖
        // Windows 目录 rename 语义。
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");
        std::fs::write(staging.join("report.json"), b"new-report").expect("new report");

        let result = publish_outputs(
            &staging,
            &output,
            &[
                (MANIFEST_NAME, MANIFEST_NAME),
                (REPORT_NAME, REPORT_NAME),
                (SURVEY_NAME, SURVEY_NAME),
            ],
        );
        assert!(result.is_err());
        assert!(
            !output.join("report.json").exists(),
            "新装入的文件在失败路径必须被删除"
        );
        assert_eq!(
            std::fs::read(output.join("manifest.toml")).expect("read"),
            b"old-manifest"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_failure_restores_original_set() {
        // #253 U2 事务式：中途失败从备份区恢复旧集合——manifest 回到旧内容，
        // survey 旧文件未被新文件替换，排除产物（network.lfca）原样恢复。
        let root = std::env::temp_dir().join(format!("lust-publish-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        std::fs::write(output.join("network.lfca"), b"stale-lfca").expect("stale");
        std::fs::write(output.join(SURVEY_NAME), b"old-survey").expect("old survey");
        // staging 只写 manifest——装入阶段在 survey 上缺源文件失败，触发恢复。
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");

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
        assert_eq!(
            std::fs::read(output.join(SURVEY_NAME)).expect("read"),
            b"old-survey",
            "未轮到的旧文件必须原样恢复"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn swap_failure_mid_backup_keeps_untouched_files() {
        // 备份阶段中途失败：尚未轮到的文件无备份条目却仍带着运行前的旧
        // 文件——回滚不得把它当新装入文件删除（判据是实际装入名单，而非
        // staged 名单）。publish 层的同类注入现在会先经过中断恢复，因此
        // 本测试直接驱动 swap_outputs/restore_backup。
        //
        // manifest 的备份目标放**目录**占位：rename 文件到已存在目录在
        // 所有平台确定性失败，备份循环在第一个名字就中断。占位目录是注入
        // 产物——真实备份中断不会留下这种条目（交付物均为常规文件），且
        // copy 语义的恢复无法归还目录——驱动 restore 前清掉它，只留真实
        // 中断现场（备份区为空）。本测试的断言只针对未触及的 report。
        let root =
            std::env::temp_dir().join(format!("lust-swap-backup-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        let backup = root.join("backup");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::create_dir_all(&backup).expect("backup");
        std::fs::write(output.join(MANIFEST_NAME), b"old-manifest").expect("old");
        std::fs::write(output.join(REPORT_NAME), b"old-report").expect("old report");
        std::fs::write(staging.join(MANIFEST_NAME), b"new-manifest").expect("new");
        std::fs::write(staging.join(REPORT_NAME), b"new-report").expect("new report");
        std::fs::create_dir_all(backup.join(MANIFEST_NAME)).expect("squat");

        let staged = [(MANIFEST_NAME, MANIFEST_NAME), (REPORT_NAME, REPORT_NAME)];
        let mut installed = Vec::new();
        let result = swap_outputs(&staging, &output, &backup, &staged, &mut installed);
        assert!(result.is_err(), "备份阶段失败必须 fail-closed");
        assert!(installed.is_empty(), "移入阶段未发生");
        std::fs::remove_dir_all(backup.join(MANIFEST_NAME)).expect("clear squat");
        restore_backup(&output, &backup, &staged, &installed).expect("restore");

        assert_eq!(
            std::fs::read(output.join(REPORT_NAME)).expect("read"),
            b"old-report",
            "备份阶段未触及的旧文件不得被回滚删除"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_recovers_crash_during_install() {
        // 装入阶段中断（残留备份有标记）：恢复旧集并删除新装入文件，
        // 随后本次 publish 正常完成、残留备份被清理。
        let root =
            std::env::temp_dir().join(format!("lust-publish-crash-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        // 上次运行崩溃现场：旧 manifest 与排除产物在备份区（带标记），
        // output 里只有装入一半的新 report。
        let stale = backup_dir(&output);
        std::fs::create_dir_all(&stale).expect("stale");
        std::fs::write(stale.join(MANIFEST_NAME), b"old-manifest").expect("old");
        std::fs::write(stale.join("network.lfca"), b"stale-lfca").expect("lfca");
        std::fs::write(stale.join(BACKUP_COMPLETE_MARKER), b"").expect("marker");
        std::fs::write(output.join(REPORT_NAME), b"crashed-report").expect("crashed");
        // 本次运行的新交付。
        std::fs::write(staging.join(MANIFEST_NAME), b"new-manifest").expect("new");
        std::fs::write(staging.join(REPORT_NAME), b"new-report").expect("new report");

        publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (REPORT_NAME, REPORT_NAME)],
        )
        .expect("publish");

        assert_eq!(
            std::fs::read(output.join(MANIFEST_NAME)).expect("read"),
            b"new-manifest"
        );
        assert_eq!(
            std::fs::read(output.join(REPORT_NAME)).expect("read"),
            b"new-report"
        );
        assert!(!output.join("network.lfca").exists(), "排除产物已清除");
        assert!(!stale.exists(), "残留备份目录已清理");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_recovers_crash_during_backup_without_touching_old_files() {
        // 备份阶段中断（残留备份无标记）：只恢复备份内容，output_dir 里
        // 未备份的旧文件原样保留——即便本次 publish 随后失败回滚，旧
        // 文件也不丢。
        let root =
            std::env::temp_dir().join(format!("lust-publish-crash-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        // 崩溃现场：旧 manifest 已进备份区（无标记），旧 report
        // 还没轮到备份、留在 output。
        let stale = backup_dir(&output);
        std::fs::create_dir_all(&stale).expect("stale");
        std::fs::write(stale.join(MANIFEST_NAME), b"old-manifest").expect("old");
        std::fs::write(output.join(REPORT_NAME), b"old-report").expect("old report");
        // 本次运行只 stage manifest——装入 report 时失败，走回滚。
        std::fs::write(staging.join(MANIFEST_NAME), b"new-manifest").expect("new");

        let result = publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (REPORT_NAME, REPORT_NAME)],
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(output.join(MANIFEST_NAME)).expect("read"),
            b"old-manifest",
            "恢复 + 回滚后 manifest 必须回到旧内容"
        );
        assert_eq!(
            std::fs::read(output.join(REPORT_NAME)).expect("read"),
            b"old-report",
            "备份阶段中断时未备份的旧文件不得被动到"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_fails_closed_on_multiple_stale_backups() {
        // 多个残留备份目录：状态歧义，fail-closed 交人工，output 不动。
        let root =
            std::env::temp_dir().join(format!("lust-publish-multistale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");
        let first = root.join(".backup-99999991-out");
        let second = root.join(".backup-99999992-out");
        std::fs::create_dir_all(&first).expect("first");
        std::fs::create_dir_all(&second).expect("second");
        std::fs::write(first.join("manifest.toml"), b"older").expect("older");

        let result = publish_outputs(&staging, &output, &[(MANIFEST_NAME, MANIFEST_NAME)]);
        let message = match result {
            Err(Error::Validation { message, .. }) => message,
            other => panic!("expected validation error, got {other:?}"),
        };
        assert!(message.contains(".backup-99999991-out"), "{message}");
        assert!(message.contains(".backup-99999992-out"), "{message}");
        assert_eq!(
            std::fs::read(output.join("manifest.toml")).expect("read"),
            b"old-manifest",
            "歧义状态下 output 必须原样保留"
        );
        assert!(first.exists() && second.exists(), "残留目录保留待人工处理");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_fails_closed_on_ambiguous_stale_backup() {
        // 残留备份有标记但 output 交付集完整：无法区分「装入末尾崩溃」
        // 与「成功后清理失败」——fail-closed，绝不静默回滚可能成功的交付。
        let root =
            std::env::temp_dir().join(format!("lust-publish-ambiguous-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"current-manifest").expect("current");
        std::fs::write(staging.join("manifest.toml"), b"new-manifest").expect("new");
        let stale = backup_dir(&output);
        std::fs::create_dir_all(&stale).expect("stale");
        std::fs::write(stale.join("manifest.toml"), b"previous-manifest").expect("prev");
        std::fs::write(stale.join(BACKUP_COMPLETE_MARKER), b"").expect("marker");

        let result = publish_outputs(&staging, &output, &[(MANIFEST_NAME, MANIFEST_NAME)]);
        let message = match result {
            Err(Error::Validation { message, .. }) => message,
            other => panic!("expected validation error, got {other:?}"),
        };
        assert!(message.contains("complete output set"), "{message}");
        assert_eq!(
            std::fs::read(output.join("manifest.toml")).expect("read"),
            b"current-manifest",
            "歧义状态下 output 必须原样保留，不得回滚"
        );
        assert!(stale.join("manifest.toml").exists(), "备份保留待人工检查");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_outputs_preserves_backup_when_rollback_fails() {
        // 回滚自身失败（新装入文件被占用等）：备份必须保留——它是旧交付
        // 集的唯一副本；错误信息指明备份位置。
        let root =
            std::env::temp_dir().join(format!("lust-publish-rollback-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join("manifest.toml"), b"old-manifest").expect("old");
        // staging 里 manifest.toml 是**目录**：装入成功（rename 目录），
        // 但回滚时 remove_file 对目录失败 → 恢复不完整。survey 缺 staging
        // 文件 → 装入阶段在 manifest 之后失败。
        std::fs::create_dir(staging.join("manifest.toml")).expect("dir");

        let result = publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (SURVEY_NAME, SURVEY_NAME)],
        );
        let message = match result {
            Err(Error::Validation { message, .. }) => message,
            other => panic!("expected validation error, got {other:?}"),
        };
        assert!(message.contains("backup preserved"), "{message}");
        let backup = backup_dir(&output);
        assert_eq!(
            std::fs::read(backup.join("manifest.toml")).expect("read"),
            b"old-manifest",
            "旧交付集的唯一副本必须保留在备份区"
        );
        assert!(
            output.join("manifest.toml").is_dir(),
            "回滚失败的现场不被破坏"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_rollback_preserves_backup_entries_for_retry() {
        // 回滚恢复用 copy 而非 rename：备份条目在恢复提交（调用方删标记
        // 与目录）前不被消耗。场景：回滚中途失败（manifest 无法归还），
        // 但 report 已先成功归还——move 语义下 report 的备份条目被消耗，
        // 下次重试恢复时「备份缺条目 ⟺ 运行前不存在」判据失效，会把
        // output 里已归还的旧 report 误判为新装入文件删除，旧交付集丢
        // 文件。copy 语义下两条备份条目都完整保留，重试幂等。
        let root = std::env::temp_dir().join(format!("lust-rollback-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(&output).expect("output");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(output.join(MANIFEST_NAME), b"old-manifest").expect("old");
        std::fs::write(output.join(REPORT_NAME), b"old-report").expect("old report");
        // staging 里 manifest.toml 是**目录**：装入成功（rename 目录），
        // 但回滚时 remove_file 对目录失败 → 恢复不完整。report 缺 staging
        // 文件 → 装入阶段在 manifest 之后失败，触发回滚。staged 顺序
        // manifest 在前：回滚先撞到失败的 manifest，report 随后成功归还。
        std::fs::create_dir(staging.join(MANIFEST_NAME)).expect("dir");

        let result = publish_outputs(
            &staging,
            &output,
            &[(MANIFEST_NAME, MANIFEST_NAME), (REPORT_NAME, REPORT_NAME)],
        );
        let message = match result {
            Err(Error::Validation { message, .. }) => message,
            other => panic!("expected validation error, got {other:?}"),
        };
        assert!(message.contains("rollback incomplete"), "{message}");

        let backup = backup_dir(&output);
        assert_eq!(
            std::fs::read(backup.join(REPORT_NAME)).expect("read"),
            b"old-report",
            "已成功归还的备份条目不得被消耗——重试恢复还要用它"
        );
        assert_eq!(
            std::fs::read(backup.join(MANIFEST_NAME)).expect("read"),
            b"old-manifest",
            "未能归还的备份条目必须保留"
        );
        assert_eq!(
            std::fs::read(output.join(REPORT_NAME)).expect("read"),
            b"old-report",
            "output 里的旧 report 已归还"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convert_recovers_interrupted_publish_before_source_verification() {
        // 中断恢复先于一切可失败操作：上次运行在 swap 中途被杀（残留带
        // 标记的备份），本次即便 source 验证就失败、根本到不了 publish，
        // output_dir 的旧交付集也已先恢复。
        let root =
            std::env::temp_dir().join(format!("lust-convert-recover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        std::fs::create_dir_all(&output).expect("output");
        // 崩溃现场：旧 manifest 在备份区（带标记），output 里只有装入
        // 一半的新 report。
        let stale = backup_dir(&output);
        std::fs::create_dir_all(&stale).expect("stale");
        std::fs::write(stale.join(MANIFEST_NAME), b"old-manifest").expect("old");
        std::fs::write(stale.join(BACKUP_COMPLETE_MARKER), b"").expect("marker");
        std::fs::write(output.join(REPORT_NAME), b"crashed-report").expect("crashed");

        let config = crate::config::LustConverterConfig {
            source_dir: PathBuf::from("E:/nonexistent-lust"),
            output_dir: output.clone(),
            converter_commit: None,
            source_bundle_url: None,
            static_bundle_url: None,
        };
        let toml = format!(
            "source_dir = 'E:/nonexistent-lust'\noutput_dir = '{}'\n",
            output.display()
        );
        let result = convert_with_config(&config, toml.as_bytes());
        assert!(
            matches!(result, Err(Error::MissingSourceFile { .. })),
            "source 验证失败: {result:?}"
        );
        assert_eq!(
            std::fs::read(output.join(MANIFEST_NAME)).expect("read"),
            b"old-manifest",
            "恢复必须发生在 source 验证之前"
        );
        assert!(
            !output.join(REPORT_NAME).exists(),
            "装入一半的新文件已被恢复删除"
        );
        assert!(!stale.exists(), "残留备份目录已清理");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn diagnostic_semantic_config_omits_static_bundle_url() {
        // 诊断模式不交付 static bundle：即便配置了 URL 也须排除在语义摘要
        // 之外（未发射资产的 URL 不得参与 configDigest）；fail-fast 模式保留。
        let config = crate::config::LustConverterConfig {
            source_dir: PathBuf::from("unused"),
            output_dir: PathBuf::from("unused"),
            converter_commit: None,
            source_bundle_url: Some("https://example.invalid/source.tar".to_owned()),
            static_bundle_url: Some("https://example.invalid/static.tar".to_owned()),
        };
        let diagnostic = semantic_config(&config, true);
        assert_eq!(diagnostic.static_bundle_url, None);
        assert_eq!(
            diagnostic.source_bundle_url.as_deref(),
            Some("https://example.invalid/source.tar")
        );
        let fail_fast = semantic_config(&config, false);
        assert_eq!(
            fail_fast.static_bundle_url.as_deref(),
            Some("https://example.invalid/static.tar")
        );
    }

    #[test]
    fn convert_fail_closed_when_output_lock_held() {
        // 另一进程（此处以同进程第二个句柄模拟）持有 output 锁时，
        // convert 在恢复/验证之前 fail-closed。
        let root = std::env::temp_dir().join(format!("lust-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        std::fs::create_dir_all(&output).expect("output");
        let guard = acquire_output_lock(&output).expect("first lock");

        let config = crate::config::LustConverterConfig {
            source_dir: PathBuf::from("E:/nonexistent-lust"),
            output_dir: output.clone(),
            converter_commit: None,
            source_bundle_url: None,
            static_bundle_url: None,
        };
        let toml = format!(
            "source_dir = 'E:/nonexistent-lust'\noutput_dir = '{}'\n",
            output.display()
        );
        let result = convert_with_config(&config, toml.as_bytes());
        assert!(
            matches!(&result, Err(Error::Validation { stage: "lock", .. })),
            "持锁冲突必须 fail-closed: {result:?}"
        );
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cleanup_failure_after_successful_swap_errors() {
        // 交付已落位但备份清理失败（备份内受管名是目录，remove_file 必败）
        // → 立即报错指明保留的备份；交付文件保持新内容、备份与标记保留。
        let root = std::env::temp_dir().join(format!("lust-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        let staging = root.join("staging");
        std::fs::create_dir_all(output.join(MANIFEST_NAME)).expect("dir as old artifact");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(staging.join(MANIFEST_NAME), b"new-manifest").expect("staged");
        let staged = [(MANIFEST_NAME, MANIFEST_NAME)];

        let error =
            publish_outputs(&staging, &output, &staged).expect_err("cleanup failure must error");
        match &error {
            Error::Validation { stage, message } => {
                assert_eq!(*stage, "cleanup");
                assert!(message.contains(".backup-"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(
            std::fs::read(output.join(MANIFEST_NAME)).expect("read"),
            b"new-manifest",
            "交付已完整落位"
        );
        let backup = backup_dir(&output);
        assert!(
            backup.join(BACKUP_COMPLETE_MARKER).exists(),
            "带标记备份保留待人工处置"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_staging_dirs_are_removed() {
        // 崩溃遗留的 staging 目录（纯新产物副本）被清理；非数字 pid 段
        // 与无关目录不动。
        let root = std::env::temp_dir().join(format!("lust-stale-staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("out");
        std::fs::create_dir_all(&output).expect("output");
        let stale = root.join(".staging-99999993-out");
        std::fs::create_dir_all(&stale).expect("stale");
        std::fs::write(stale.join("manifest.toml"), b"staged").expect("write");
        let non_numeric = root.join(".staging-abc-out");
        std::fs::create_dir_all(&non_numeric).expect("non-numeric");
        let unrelated = root.join("other");
        std::fs::create_dir_all(&unrelated).expect("unrelated");

        remove_stale_staging(&output);

        assert!(!stale.exists(), "残留 staging 已清理");
        assert!(non_numeric.exists(), "非数字 pid 段不匹配，不动");
        assert!(unrelated.exists(), "无关目录不动");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn publish_outputs_resolves_symlinked_output_dir() {
        // output_dir 是符号链接：调用方先锚定（生产路径在持锁前解析一次），
        // staging/backup 落在真实目录旁，publish 的 rename 不跨设备（词法
        // parent 方案在链接与目标跨盘时必 EXDEV）。
        let root =
            std::env::temp_dir().join(format!("lust-publish-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(&real).expect("real");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let anchor = output_anchor(&link, &root);
        let staging = staging_dir(&anchor);
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(staging.join(MANIFEST_NAME), b"new-manifest").expect("new");

        publish_outputs(&staging, &anchor, &[(MANIFEST_NAME, MANIFEST_NAME)]).expect("publish");

        assert_eq!(
            std::fs::read(real.join(MANIFEST_NAME)).expect("read"),
            b"new-manifest",
            "交付必须落到符号链接的真实目标"
        );
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
