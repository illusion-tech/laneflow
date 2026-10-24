use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use laneflow_compiler::{CompileLimits, derive_canonical_stable_id_v1};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, TrafficWorld, WorldConfig,
    WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, ParticipantStreamOrdinal, RightOfWayPolicySetId,
    SignalGroupId, SignalGroupOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_urban_generator::{Catalog, Scale};
use serde::{Deserialize, Serialize};

use crate::{Result, checked, invalid, sha256};

/// 生成器清单中的文件字节数与 SHA-256：属于 urban-generator 的制品清单（含可移植
/// 规范制品家族，ADR 0033 §9），harness 只按清单核对并照抄，不改算法（#1042）。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArtifactFileDigest {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Manifest {
    manifest_version: u32,
    scale: String,
    tiles: u32,
    nominal_individuals: u32,
    fixed_step_ms: u64,
    network_revision: String,
    files: BTreeMap<String, ArtifactFileDigest>,
}

pub struct Artifacts {
    pub(crate) catalog: Catalog,
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) files: BTreeMap<String, ArtifactFileDigest>,
    pub(crate) manifest_digest: String,
    pub(crate) individuals: u32,
    pub(crate) tiles: u32,
    pub(crate) dt: u64,
    pub(crate) edges: BTreeMap<String, LaneEdgeOrdinal>,
}

impl Artifacts {
    /// Borrows the catalog bound to the loaded source digests.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Borrows the shared network bound to the loaded source digests.
    pub fn revision(&self) -> &Arc<SharedNetworkRevision> {
        &self.revision
    }

    /// Loads the generator's real checked LFCA. No topology is synthesized by the runner.
    pub fn load(directory: &Path) -> Result<Self> {
        Self::load_with_spatial_option(directory, SpatialBuildOption::Omit)
    }

    /// Load the same checked inputs with canonical geometry for Adapter observation.
    #[cfg(feature = "adapter")]
    pub fn load_spatial(directory: &Path) -> Result<Self> {
        Self::load_with_spatial_option(directory, SpatialBuildOption::RetainAvailable)
    }

    fn load_with_spatial_option(directory: &Path, spatial: SpatialBuildOption) -> Result<Self> {
        let manifest_bytes = fs::read(directory.join("manifest.toml"))?;
        let manifest: Manifest = toml::from_str(
            std::str::from_utf8(&manifest_bytes).map_err(|e| invalid(e.to_string()))?,
        )?;
        let scale = checked("artifact scale", Scale::parse(&manifest.scale))?;
        if manifest.manifest_version != 1
            || manifest.tiles != scale.tile_count()
            || manifest.nominal_individuals != scale.nominal_individual_count()
            || manifest.fixed_step_ms != scale.fixed_step_ms()
        {
            return Err(invalid("artifact shape differs from its fixed scale"));
        }
        let mut consumed = BTreeMap::new();
        let mut read = |name: &str| -> Result<Vec<u8>> {
            let bytes = fs::read(directory.join(name))?;
            let actual = ArtifactFileDigest {
                bytes: bytes.len() as u64,
                sha256: sha256(&bytes),
            };
            if let Some(expected) = manifest.files.get(name) {
                if expected != &actual {
                    probe_artifact_mismatch(&directory.join(name), name, expected, &bytes);
                    return Err(invalid(format!("artifact digest mismatch: {name}")));
                }
            } else {
                return Err(invalid(format!("artifact digest mismatch: {name}")));
            }
            consumed.insert(name.to_owned(), actual);
            Ok(bytes)
        };
        let catalog: Catalog = toml::from_str(
            std::str::from_utf8(&read("routes.toml")?).map_err(|e| invalid(e.to_string()))?,
        )?;
        let _config = read("config.toml")?;
        for name in ["common.lfre", "topology.lfre"] {
            let _ = read(name)?;
        }
        let bytes = read("network.lfca")?;
        let input = checked(
            "LFCA check",
            check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD),
        )?;
        let revision = checked(
            "shared network",
            build_shared_network_revision(
                input,
                SharedNetworkBuildOptions::new(
                    spatial,
                    SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648),
                ),
            ),
        )?;
        if catalog.catalog_version != 2
            || catalog.scale != manifest.scale
            || catalog.network_revision != manifest.network_revision
            || format!(
                "{:x}",
                revision.canonical_origin().network_revision().as_digest()
            ) != manifest.network_revision
        {
            return Err(invalid("catalog and LFCA identity differ"));
        }
        let mut edges = BTreeMap::new();
        for (key, id) in &catalog.edge_ids {
            let id: LaneEdgeId = crate::catalog_id(id)?;
            let ordinal = revision
                .identity()
                .ordinal(id)
                .ok_or_else(|| invalid(format!("unknown edge {key}")))?;
            edges.insert(key.clone(), ordinal);
        }
        Ok(Self {
            catalog,
            revision,
            files: consumed,
            manifest_digest: crate::digest::Domain::File.hex(&manifest_bytes),
            individuals: manifest.nominal_individuals,
            tiles: manifest.tiles,
            dt: manifest.fixed_step_ms,
            edges,
        })
    }

    /// #538 起装机统一走 `install_with_route_slack`（标准路径传 0）；路线槽、
    /// 边占用与冲突占用三项容量在目录基础上按 `extra_routes` 条最坏路线同步加
    /// 余量，供 candidate / admitted 注册采样；vehicle_capacity 与固定步长不变。
    /// 容量进入快照与逻辑摘要：余量装机与标准装机不同身份，仅自含测量世界使用。
    pub(crate) fn install_with_route_slack(
        &self,
        execution: laneflow_runtime::ExecutionConfig,
        extra_routes: u32,
    ) -> Result<TrafficWorld> {
        let pin: RightOfWayPolicySetId = crate::catalog_id(&self.catalog.policy_id)?;
        let mut config = self.world_config()?;
        // 余量按最坏路线计：边占用取目录路线的最大边数；冲突占用取目录路线的
        // 逐边 passages 之和的最大值（与 world_config 的路线级累加同口径——注册
        // 一条路线的冲突消耗是其全部 hop 的 passages 之和，不是单边最大值）。
        let passages = self.conflict_passages_per_edge();
        let mut max_edges = 0_u64;
        let mut max_conflict = 0_u64;
        for route in &self.catalog.routes {
            let edges = route.edge_keys.len() as u64;
            max_edges = max_edges.max(edges);
            let mut route_conflict = 0_u64;
            for key in &route.edge_keys {
                if let Some(edge) = self.edges.get(key) {
                    route_conflict += passages.get(edge).copied().unwrap_or(0);
                }
            }
            max_conflict = max_conflict.max(route_conflict);
        }
        config = config.with_added_route_capacity(
            extra_routes,
            u64::from(extra_routes) * max_edges,
            u64::from(extra_routes) * max_conflict,
        );
        let world = checked(
            "world install",
            TrafficWorld::install(
                self.revision.clone(),
                config,
                execution,
                self.committed_source()?,
                544,
                WorldPolicySelection::Pinned(PolicyPin { policy: pin }),
            ),
        )?;
        self.check_policy(&world)?;
        Ok(world)
    }

    /// 每条 LaneEdge 的冲突占用数（participant stream 的 passages 计入其路径边）。
    fn conflict_passages_per_edge(&self) -> BTreeMap<LaneEdgeOrdinal, u64> {
        let mut passages = BTreeMap::new();
        for raw in 0..self
            .revision
            .identity()
            .entity_count(EntityKind::ParticipantStream)
        {
            let stream = self
                .revision
                .conflict()
                .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
                .expect("checked stream");
            let path = self
                .revision
                .traffic()
                .maneuvers()
                .maneuver_path(stream.maneuver_path())
                .expect("checked path");
            passages.insert(path.edges()[1], stream.passages().len() as u64);
        }
        passages
    }

    /// 安装与续跑恢复共用的世界容量配置。
    pub(crate) fn world_config(&self) -> Result<WorldConfig> {
        let passages = self.conflict_passages_per_edge();
        let edge_capacity = self
            .catalog
            .routes
            .iter()
            .map(|r| r.edge_keys.len() as u64)
            .sum();
        let mut conflict_capacity = 0_u64;
        for route in &self.catalog.routes {
            for key in &route.edge_keys {
                let edge = self
                    .edges
                    .get(key)
                    .ok_or_else(|| invalid(format!("unknown route edge {key}")))?;
                conflict_capacity += passages.get(edge).copied().unwrap_or(0);
            }
        }
        Ok(WorldConfig::new(
            self.individuals,
            self.catalog.routes.len() as u32,
            edge_capacity,
            conflict_capacity,
            self.dt,
        ))
    }

    /// 安装与续跑恢复共用的已提交来源。
    pub(crate) fn committed_source(&self) -> Result<CommittedNetworkSource> {
        let origin = self.revision.canonical_origin();
        let source = checked(
            "source reference",
            PublishedLfcaReference::new(
                "fixture://lf-cn-urban-v2",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            ),
        )?;
        Ok(CommittedNetworkSource::Published { reference: source })
    }

    /// 世界（含快照恢复所得）必须使用 urban-conservative-v1 间隙策略。
    pub(crate) fn check_policy(&self, world: &TrafficWorld) -> Result<()> {
        if world.policy_gap_profiles().is_empty()
            || world
                .policy_gap_profiles()
                .iter()
                .any(|g| g.required_lead_ms() != 5_500 + self.dt || g.required_lag_ms() != 2_500)
        {
            return Err(invalid("urban-conservative-v1 gap differs"));
        }
        Ok(())
    }

    pub(crate) fn signal_group(&self, key: &str) -> Result<SignalGroupOrdinal> {
        let id = derive_canonical_stable_id_v1(
            EntityKind::SignalGroup,
            &self.catalog.namespace,
            key,
            &CompileLimits::single_network_1m_v2(),
        )
        .map_err(|error| invalid(format!("signal group identity: {error:?}")))?;
        self.revision
            .identity()
            .ordinal(SignalGroupId::from_untyped(id))
            .ok_or_else(|| invalid(format!("unknown signal group {key}")))
    }
}

/// 按生成器清单的口径（SHA-256）计算文件摘要，供运行期间核对输入制品未变。
#[cfg(feature = "adapter")]
pub(crate) fn artifact_file_digest(path: &Path) -> Result<ArtifactFileDigest> {
    let bytes = fs::read(path)?;
    Ok(ArtifactFileDigest {
        bytes: bytes.len() as u64,
        sha256: sha256(&bytes),
    })
}

/// 工件摘要失配现场探针（#794 症状③）：输出清单期望值与读回实际值的长度差、摘要
/// 差异、读回头部 32 字节及流式二次回读对照；另输出全文件 64 等分块摘要，供仅 load
/// 路径命中（写后回读干净、无内容参考）时与健康参考离线逐块对照定位失真块。
/// 仅诊断输出：命中不改变 `Artifacts::load` 的失败语义与错误文案。
fn probe_artifact_mismatch(
    path: &Path,
    name: &str,
    expected: &ArtifactFileDigest,
    readback: &[u8],
) {
    let (reread_same, reread_sha) = stream_reread(path, readback);
    eprintln!(
        "[digest-probe] site=artifact-load name={name} expected_bytes={} expected_sha={} readback_bytes={} readback_sha={} len_delta={} head32={:02x?} reread_same={} reread_sha={}",
        expected.bytes,
        expected.sha256,
        readback.len() as u64,
        sha256(readback),
        readback.len() as i64 - expected.bytes as i64,
        &readback[..readback.len().min(32)],
        reread_same,
        reread_sha,
    );
    let block_count = 64_usize;
    let block_len = readback.len().div_ceil(block_count).max(1);
    let block_shas: Vec<String> = readback.chunks(block_len).map(sha256).collect();
    eprintln!(
        "[digest-probe] site=artifact-load name={name} block_len={block_len} block_shas={}",
        block_shas.join(","),
    );
}

/// 流式二次回读（#953 审阅）：与首次读回逐窗口比对并累计摘要，不整体持有第二份
/// 副本（100k 档工件达数百 MiB，失败瞬间的额外整份分配可自致 OOM）。
fn stream_reread(path: &Path, first: &[u8]) -> (bool, String) {
    use sha2::{Digest, Sha256};
    const WINDOW: usize = 64 * 1024;
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return (false, "unreadable".to_owned()),
    };
    let mut hasher = Sha256::new();
    let mut window = [0_u8; WINDOW];
    let mut offset = 0_usize;
    let mut same = true;
    loop {
        let read = match std::io::Read::read(&mut file, &mut window) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return (false, "unreadable".to_owned()),
        };
        hasher.update(&window[..read]);
        if offset + read > first.len() || first[offset..offset + read] != window[..read] {
            same = false;
        }
        offset += read;
    }
    let sha = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (same && offset == first.len(), sha)
}
