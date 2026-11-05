//! #1271 拓扑核查：横向过渡段汇入窗口内的平行边盲区扫描。
//!
//! #1271 的缺口形制（fixture `straight_vehicle_on_parallel_edge_is_a_registered_gap`）：
//! 目标车道在过渡段汇入点之前分岔；过渡车经 `T` 汇入目标车道，直行车走与 `T`
//! 平行的边后驶向分岔支路；双方同拍到达同一位置且互不可见。运行时根因是
//! P4 只检查路线是否经过本过渡段或其前驱、后继（`kernel/transition_visibility.rs`
//! 的邻域口径），因此本核查按同一口径定义盲区：
//!
//! 1. 枚举全部横向过渡段（与运行时同判据 `is_lateral_transition_segment`）；
//! 2. 对每条过渡段 `T`，取可见邻域 `N(T) = {T} ∪ 前驱 ∪ 后继`；
//! 3. 以 `T` 的长度在邻接编制车道的纵向链上确定汇入窗口
//!    `[前驱链末, +len(T))`；
//! 4. 同路段（section）内纵向跨度与窗口相交、且不属于 `N(T)` 的编制车道边是
//!    平行边候选；
//! 5. 存在「不经过 `N(T)` 任何边、却经过平行边候选」的已注册路线即为
//!    违规（P4 看不见的直行车实际在跑）；汇入点上游出现分岔（后继跨多条
//!    编制车道或多后继）记录为使能结构。
//!
//! 本核查只读静态包，不构造世界、不推进 tick；不含横向几何判定（运行时同样
//! 没有），纵向窗口按编制车道边链累计毫米长度计算。含违规拓扑的路网不得
//! 用于安全验收（#1271 处置）；本核查的结论记录供 `manifest` 的 L1 挂接消费。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{LaneEdgeId, LaneEdgeOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_synth_generator::InitialState;
use serde::{Deserialize, Serialize};

use crate::{Error, invalid};

/// 核查记录的格式身份。
pub const L1_RECORD_FORMAT: &str = "lf-l1-topology-check-1";

/// 输入包的四个规范文件名（与 `laneflow-synth-generator` 文件合同一致）。
const BUNDLE_FILES: [&str; 4] = [
    "input-plan.json",
    "manifest.json",
    "network.lfca",
    "state.json",
];

/// 从输入包目录读取核查所需文件；不做摘要重算（身份原样携带自 manifest）。
#[derive(Debug)]
pub struct BundleInput {
    pub identity: String,
    pub manifest: serde_json::Value,
    pub network_lfca: Vec<u8>,
    pub state_json: Vec<u8>,
}

/// 读取一个 LF-SYNTH 输入包目录。
///
/// # Errors
/// 目录文件不齐、`manifest.json` 无法解析或身份字段缺失时失败。
pub fn load_bundle(directory: &Path) -> Result<BundleInput, Error> {
    for name in BUNDLE_FILES {
        if !directory.join(name).is_file() {
            return Err(invalid(format!(
                "输入包缺少 {name}：{}",
                directory.display()
            )));
        }
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.join("manifest.json"))
            .map_err(|e| invalid(format!("manifest.json 读取失败：{e}")))?,
    )
    .map_err(|e| invalid(format!("manifest.json 解析失败：{e}")))?;
    let scale = manifest
        .pointer("/identity/scale")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("manifest.json 缺少 identity.scale"))?;
    let case = manifest
        .pointer("/identity/case")
        .cloned()
        .ok_or_else(|| invalid("manifest.json 缺少 identity.case"))?;
    let case_name = match &case {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => map
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "case".to_owned()),
        _ => "case".to_owned(),
    };
    let delta = manifest
        .pointer("/identity/fixed_delta_ms")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("manifest.json 缺少 identity.fixed_delta_ms"))?;
    Ok(BundleInput {
        identity: format!("{scale}-{case_name}-{delta}"),
        manifest,
        network_lfca: fs::read(directory.join("network.lfca"))
            .map_err(|e| invalid(format!("network.lfca 读取失败：{e}")))?,
        state_json: fs::read(directory.join("state.json"))
            .map_err(|e| invalid(format!("state.json 读取失败：{e}")))?,
    })
}

/// 一条编制车道边链及其纵向跨度。
#[derive(Debug)]
struct LaneChain {
    edges: Vec<(LaneEdgeOrdinal, u64, u64)>,
}

/// 逐过渡段核查结果。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionReport {
    pub edge: String,
    pub window_mm: (u64, u64),
    pub predecessors: Vec<String>,
    pub successors: Vec<String>,
    /// 纵向跨度与汇入窗口相交、且不在可见邻域内的编制车道边（信息项；
    /// 平行边本身不构成违规，配合盲路线才构成）。
    pub parallel_edges: Vec<ParallelEdgeReport>,
    /// 不经过可见邻域却经过平行边的已注册路线（违规）。
    pub blind_routes: Vec<BlindRouteReport>,
    /// 汇入点上游的分岔使能结构描述。
    pub upstream_forks: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParallelEdgeReport {
    pub edge: String,
    pub span_mm: (u64, u64),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlindRouteReport {
    pub route_rank: u32,
    pub parallel_edge: String,
    pub edge_count: usize,
}

/// 核查记录（`lf-l1-topology-check-1`）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct L1Report {
    pub format: String,
    pub bundle: BundleReport,
    pub lane_edge_count: u32,
    pub route_count: usize,
    pub transitions: Vec<TransitionReport>,
    pub verdict: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BundleReport {
    pub identity: String,
    /// manifest 声明的身份原像（逐字段原样携带，不做摘要）。
    pub manifest_identity: serde_json::Value,
    pub source_commit: Option<String>,
    pub generator_commit: Option<String>,
    pub config_digest: Option<String>,
}

/// 对一个输入包执行核查。
///
/// # Errors
/// LFCA 受检读取、共享路网构建、路线身份解析失败，或车道边链不成线性时失败。
pub fn check(input: &BundleInput) -> Result<L1Report, Error> {
    let checked = check_canonical_network_input(input.network_lfca.as_slice(), FormatLimits::HARD)
        .map_err(|e| invalid(format!("LFCA 受检读取失败：{e:?}")))?;
    let revision = build_shared_network_revision(
        checked,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 2 * 1_024 * 1_024 * 1_024),
        ),
    )
    .map_err(|e| invalid(format!("共享路网构建失败：{e:?}")))?;
    let state: InitialState = serde_json::from_slice(&input.state_json)?;
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let identity = revision.identity();
    let lengths = traffic.lane_lengths_millimetres();

    let name = |edge: LaneEdgeOrdinal| -> String {
        identity
            .stable_id(edge)
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("lane-edge#{}", edge.index()))
    };

    // 路线目录：edge_ids（lfid1_ 文本）→ LaneEdgeOrdinal。
    let mut routes: Vec<(u32, Vec<LaneEdgeOrdinal>)> = Vec::with_capacity(state.routes.len());
    for route in &state.routes {
        let mut edges = Vec::with_capacity(route.edge_ids.len());
        for id in &route.edge_ids {
            let parsed = id
                .parse::<LaneEdgeId>()
                .map_err(|e| invalid(format!("路线边身份解析失败 {id}：{e}")))?;
            let ordinal = identity
                .ordinal(parsed)
                .ok_or_else(|| invalid(format!("路线边不在共享路网中：{id}")))?;
            edges.push(ordinal);
        }
        routes.push((route.route_rank, edges));
    }

    // 编制车道 → 线性边链（含纵向毫米跨度）。分岔/多头的车道记为使能结构。
    let mut lane_edges: BTreeMap<usize, Vec<LaneEdgeOrdinal>> = BTreeMap::new();
    let mut lane_of_edge: BTreeMap<usize, usize> = BTreeMap::new();
    let mut structural_findings: Vec<String> = Vec::new();
    for raw in 0..traffic.lane_edge_count() {
        let edge = LaneEdgeOrdinal::from_raw(raw);
        if let Some(lane) = relations.lane_edge_authoring_lane(edge) {
            lane_edges.entry(lane.index()).or_default().push(edge);
            lane_of_edge.insert(edge.index(), lane.index());
        }
    }
    let mut chains: BTreeMap<usize, LaneChain> = BTreeMap::new();
    for (lane, edges) in &lane_edges {
        let member = |edge: &LaneEdgeOrdinal| lane_of_edge.get(&edge.index()) == Some(lane);
        let mut heads = Vec::new();
        for edge in edges {
            let has_lane_predecessor = traffic
                .predecessors(*edge)
                .is_some_and(|list| list.iter().any(member));
            if !has_lane_predecessor {
                heads.push(*edge);
            }
        }
        if heads.len() != 1 {
            structural_findings.push(format!(
                "编制车道 #{lane} 有 {} 个起点（汇入式车道起点），按多起点的使能结构登记",
                heads.len()
            ));
        }
        let mut chain: Vec<(LaneEdgeOrdinal, u64, u64)> = Vec::new();
        let mut cursor = heads.first().copied();
        let mut offset: u64 = 0;
        while let Some(edge) = cursor {
            let length = u64::from(lengths.get(edge.index()).copied().unwrap_or(0));
            chain.push((edge, offset, offset + length));
            offset += length;
            let nexts: Vec<LaneEdgeOrdinal> = traffic.successors(edge).map_or(Vec::new(), |list| {
                list.iter().copied().filter(member).collect()
            });
            if nexts.len() > 1 {
                structural_findings.push(format!(
                    "编制车道 #{lane} 在 {} 处分岔（{} 条同车道后继），按使能结构登记",
                    name(edge),
                    nexts.len()
                ));
            }
            cursor = nexts.first().copied();
        }
        if chain.len() != edges.len() {
            return Err(invalid(format!(
                "编制车道 #{lane} 边链不成线性（{} 条边只链到 {} 条），无法计算纵向窗口",
                edges.len(),
                chain.len()
            )));
        }
        chains.insert(*lane, LaneChain { edges: chain });
    }

    let chain_end = |edge: LaneEdgeOrdinal| -> Option<u64> {
        let lane = lane_of_edge.get(&edge.index())?;
        chains
            .get(lane)?
            .edges
            .iter()
            .find(|(member, _, _)| *member == edge)
            .map(|(_, _, end)| *end)
    };
    let section_of_edge = |edge: LaneEdgeOrdinal| -> Option<usize> {
        relations
            .lane_edge_authoring_lane(edge)
            .and_then(|lane| relations.authoring_section(lane))
            .map(|section| section.index())
    };

    let mut transitions = Vec::new();
    for raw in 0..traffic.lane_edge_count() {
        let transition = LaneEdgeOrdinal::from_raw(raw);
        if !traffic.is_lateral_transition_segment(transition) {
            continue;
        }
        let length = u64::from(lengths.get(transition.index()).copied().unwrap_or(0));
        let preds: Vec<LaneEdgeOrdinal> = traffic
            .predecessors(transition)
            .map_or(Vec::new(), |list| list.to_vec());
        let succs: Vec<LaneEdgeOrdinal> = traffic
            .successors(transition)
            .map_or(Vec::new(), |list| list.to_vec());
        let mut neighborhood: Vec<usize> = vec![transition.index()];
        neighborhood.extend(preds.iter().map(|e| e.index()));
        neighborhood.extend(succs.iter().map(|e| e.index()));
        let in_neighborhood = |edge: &LaneEdgeOrdinal| neighborhood.contains(&edge.index());

        // 汇入窗口：沿前渡链回溯到编制车道边，取其链末为窗口起点。
        let mut window: Option<(u64, u64)> = None;
        for pred in &preds {
            let mut origin = *pred;
            let mut walked = 0_u64;
            loop {
                if relations.lane_edge_authoring_lane(origin).is_some() {
                    if let Some(end) = chain_end(origin) {
                        window = Some((end + walked, end + walked + length));
                    }
                    break;
                }
                let Some(previous) = traffic
                    .predecessors(origin)
                    .and_then(|list| list.first().copied())
                else {
                    break;
                };
                walked += u64::from(lengths.get(origin.index()).copied().unwrap_or(0));
                origin = previous;
            }
            if window.is_some() {
                break;
            }
        }
        let Some(window) = window else {
            return Err(invalid(format!(
                "过渡段 {} 找不到编制车道前渡链，无法确定汇入窗口",
                name(transition)
            )));
        };

        // 平行边候选：前渡/后渡车道所在路段内，跨度与窗口相交且不在邻域内。
        let mut sections: Vec<usize> = Vec::new();
        for edge in preds.iter().chain(succs.iter()) {
            if let Some(section) = section_of_edge(*edge)
                && !sections.contains(&section)
            {
                sections.push(section);
            }
        }
        let mut parallel: Vec<(usize, ParallelEdgeReport)> = Vec::new();
        for chain in chains.values() {
            for (edge, start, end) in &chain.edges {
                if in_neighborhood(edge) {
                    continue;
                }
                let Some(section) = section_of_edge(*edge) else {
                    continue;
                };
                if !sections.contains(&section) {
                    continue;
                }
                if *start < window.1 && window.0 < *end {
                    parallel.push((
                        edge.index(),
                        ParallelEdgeReport {
                            edge: name(*edge),
                            span_mm: (*start, *end),
                        },
                    ));
                }
            }
        }

        // 盲路线：不经过邻域任何边、却经过平行边候选。
        let parallel_set: Vec<usize> = parallel.iter().map(|(index, _)| *index).collect();
        let mut blind: Vec<BlindRouteReport> = Vec::new();
        for (rank, edges) in &routes {
            if edges.iter().any(in_neighborhood) {
                continue;
            }
            if let Some(hit) = edges
                .iter()
                .find(|edge| parallel_set.contains(&edge.index()))
            {
                blind.push(BlindRouteReport {
                    route_rank: *rank,
                    parallel_edge: name(*hit),
                    edge_count: edges.len(),
                });
            }
        }

        // 汇入点上游分岔：后渡编制车道链上、后渡边之前的边存在跨车道/多后继。
        let mut forks: Vec<String> = Vec::new();
        for succ in &succs {
            let Some(lane) = relations.lane_edge_authoring_lane(*succ) else {
                continue;
            };
            let Some(chain) = chains.get(&lane.index()) else {
                continue;
            };
            for (edge, _, end) in &chain.edges {
                if *edge == *succ {
                    break;
                }
                let nexts: &[LaneEdgeOrdinal] = traffic.successors(*edge).map_or(&[], |list| list);
                let mut lanes: Vec<usize> = Vec::new();
                for next in nexts {
                    if let Some(next_lane) = relations.lane_edge_authoring_lane(*next)
                        && !lanes.contains(&next_lane.index())
                    {
                        lanes.push(next_lane.index());
                    }
                }
                if nexts.len() > 1 || lanes.len() > 1 {
                    forks.push(format!(
                        "后继车道链在 {}（链末 {} mm）存在分岔：{} 条后继跨 {} 条编制车道",
                        name(*edge),
                        end,
                        nexts.len(),
                        lanes.len()
                    ));
                }
            }
        }

        transitions.push(TransitionReport {
            edge: name(transition),
            window_mm: window,
            predecessors: preds.iter().map(|edge| name(*edge)).collect(),
            successors: succs.iter().map(|edge| name(*edge)).collect(),
            parallel_edges: parallel.into_iter().map(|(_, report)| report).collect(),
            blind_routes: blind,
            upstream_forks: forks,
        });
    }

    structural_findings.extend(
        transitions
            .iter()
            .flat_map(|report| report.upstream_forks.iter().cloned())
            .collect::<Vec<_>>(),
    );
    let blind_count: usize = transitions.iter().map(|t| t.blind_routes.len()).sum();
    let verdict = if blind_count == 0 && structural_findings.is_empty() {
        "clean"
    } else {
        "violations-found"
    };
    Ok(L1Report {
        format: L1_RECORD_FORMAT.to_owned(),
        bundle: BundleReport {
            identity: input.identity.clone(),
            manifest_identity: input
                .manifest
                .get("identity")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            source_commit: input
                .manifest
                .get("source_commit")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            generator_commit: input
                .manifest
                .get("generator_commit")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            config_digest: input
                .manifest
                .get("config_digest")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        },
        lane_edge_count: traffic.lane_edge_count(),
        route_count: routes.len(),
        transitions,
        verdict: verdict.to_owned(),
    })
}
