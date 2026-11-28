use laneflow_compiler::derive_canonical_stable_id_v1;
use laneflow_static_contract::EntityKind;
use serde::{Deserialize, Serialize};

use crate::network_lane_change::{LC_EDIT_NAMESPACE, LC_ROUTES, cluster_key};
use crate::{Case, Error, Identity, NAMESPACE, POLICY_KEY, compile_error, key};

/// 由稳定标识引用静态实体的初始状态目录。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InitialState {
    pub format: String,
    pub identity: Identity,
    pub policy_id: String,
    pub vehicle_profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_driving: Option<HandoffDrivingInput>,
    pub routes: Vec<RouteRecord>,
    pub vehicles: Vec<VehicleRecord>,
}

/// 场景给定的净空限速曲线及几何求证设置。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandoffDrivingInput {
    /// 点的两项依次为净空毫米及速度上限毫米每秒。
    pub curve: Vec<[u32; 2]>,
    /// 净空限速段的最大有向站位跨度。
    pub max_span_mm: u32,
    /// 后轴三维行程求界的容差。
    pub travel_tolerance_mm: u32,
}

/// 已注册的路线出现、共边，以及首拍申请的候选。所有站位均为走廊毫米。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandoffInput {
    /// LFCA 发布的区段稳定标识。
    pub section_id: String,
    /// 共边左侧车道在该区段有序成员中的下标。
    pub left_index: u32,
    /// 本路线中交接源边的出现下标。
    pub source_index: u32,
    /// 允许前杠交接的站位闭区间。
    pub stations_mm: [u64; 2],
    /// 整个候选车体可以投影的站位闭区间。
    pub station_window_mm: [u64; 2],
    /// 出生时的后轴起、止站位，重试时按本车已行距离平移。
    pub rear_stations_mm: [u64; 2],
    /// 转向速率与制动求证使用的最高速度。
    pub speed_mm_s: u32,
}

/// 路线的有向 edge 序列；数组顺序就是 occurrence 顺序。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteRecord {
    pub route_rank: u32,
    pub cell: u32,
    pub route: u32,
    pub edge_ids: Vec<String>,
    /// 有信号 Gate 的路线携带信号组稳定标识；无信号准入路线为 `None`（L 族）。
    pub signal_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<HandoffInput>,
}

/// logical rank 独立于 Runtime handle；progress 是 edge progress，单位毫米。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VehicleRecord {
    pub logical_rank: u32,
    pub route_rank: u32,
    pub slot: u32,
    pub status: String,
    pub edge_occurrence: u32,
    pub progress_mm: u32,
    pub speed_mm_per_s: u32,
    /// 停车空间稳定标识；换道者（L 族，非停车语义）为 `None`。
    pub parking_space_id: Option<String>,
}

pub(crate) fn stable_id(kind: EntityKind, source_key: &str) -> Result<String, Error> {
    stable_id_in(NAMESPACE, kind, source_key)
}

pub(crate) fn stable_id_in(
    namespace: &str,
    kind: EntityKind,
    source_key: &str,
) -> Result<String, Error> {
    let id = derive_canonical_stable_id_v1(kind, namespace, source_key)
        .map_err(|error| compile_error("stable identity", error))?;
    Ok(format!("lfid1_{}_{id:x}", kind.slug()))
}

pub(crate) fn initial_state(
    identity: &Identity,
    section_ids: &[String],
) -> Result<InitialState, Error> {
    let mut routes = Vec::new();
    let mut vehicles = Vec::new();
    for cell in 0..identity.cells() {
        for route in 0..4 {
            let route_rank = cell * 4 + route;
            routes.push(RouteRecord {
                route_rank,
                cell,
                route,
                edge_ids: vec![
                    stable_id(EntityKind::LaneEdge, &key(cell, route, "entry"))?,
                    stable_id(EntityKind::LaneEdge, &key(cell, route, "exit"))?,
                ],
                handoff: None,
                // L1 背景无信号门（D1：L 族自有背景语义），信号组为 `None`。
                signal_group_id: if identity.lane_change().is_some() {
                    None
                } else {
                    Some(stable_id(
                        EntityKind::SignalGroup,
                        &key(cell, route, "group"),
                    )?)
                },
            });
            for slot in 0..25 {
                let active = match &identity.case {
                    Case::W1 | Case::W4 | Case::LaneChange(_) => {
                        route < 2 || slot < if route == 2 { 12 } else { 13 }
                    }
                    Case::W2 => true,
                    Case::W3 => slot < if route == 3 { 7 } else { 6 },
                };
                let dense = identity.case == Case::W2 || (identity.case != Case::W3 && route < 2);
                vehicles.push(VehicleRecord {
                    logical_rank: cell * 100 + route * 25 + slot,
                    route_rank,
                    slot,
                    status: if active { "active" } else { "parked" }.into(),
                    edge_occurrence: if active { 0 } else { 1 },
                    progress_mm: if active {
                        20_000 + slot * if dense { 6_500 } else { 28_000 }
                    } else {
                        5_000 + 7_500 * slot
                    },
                    speed_mm_per_s: if active && !dense { 13_900 } else { 0 },
                    parking_space_id: Some(stable_id(
                        EntityKind::ParkingSpace,
                        &key(cell, route, &format!("space{slot:02}")),
                    )?),
                });
            }
        }
    }
    if let Some(params) = identity.lane_change() {
        if section_ids.len() != params.clusters as usize {
            return Err(Error::Invalid("换道区段身份的数量与换道簇不一致".into()));
        }
        for (cluster, section_id) in section_ids.iter().enumerate() {
            let cluster = cluster as u32;
            for (slot, keys) in LC_ROUTES.iter().enumerate() {
                let speed = crate::HANDOFF_SPEED_MM_S[slot];
                let slot = slot as u32;
                let rank = cluster * params.changers_per_cluster + slot;
                let route_rank = identity.cells() * 4 + rank;
                routes.push(RouteRecord {
                    route_rank,
                    cell: identity.cells() + cluster,
                    route: slot,
                    edge_ids: keys
                        .iter()
                        .map(|base| {
                            stable_id_in(
                                LC_EDIT_NAMESPACE,
                                EntityKind::LaneEdge,
                                &cluster_key(base, cluster),
                            )
                        })
                        .collect::<Result<Vec<_>, Error>>()?,
                    signal_group_id: None,
                    handoff: Some(HandoffInput {
                        section_id: section_id.clone(),
                        left_index: slot,
                        source_index: 0,
                        stations_mm: [20_000, 110_000],
                        station_window_mm: [0, 120_000],
                        rear_stations_mm: [10_000, 40_000],
                        speed_mm_s: 3_000,
                    }),
                });
                vehicles.push(VehicleRecord {
                    logical_rank: identity.scale + rank,
                    route_rank,
                    slot,
                    status: "active".into(),
                    edge_occurrence: 0,
                    progress_mm: crate::HANDOFF_FRONT_MM,
                    speed_mm_per_s: speed,
                    parking_space_id: None,
                });
            }
        }
    }
    Ok(InitialState {
        format: "lf-synth-state-1".into(),
        identity: identity.clone(),
        policy_id: if identity.lane_change().is_some() {
            stable_id_in(LC_EDIT_NAMESPACE, EntityKind::RightOfWayPolicySet, "policy")?
        } else {
            stable_id(EntityKind::RightOfWayPolicySet, POLICY_KEY)?
        },
        handoff_driving: identity.lane_change().map(|_| HandoffDrivingInput {
            curve: vec![[0, 1_000], [100, laneflow_static_contract::MAX_SPEED_MM_S]],
            max_span_mm: 3_000,
            travel_tolerance_mm: 5,
        }),
        vehicle_profile_id: stable_id(EntityKind::VehicleProfile, "car")?,
        routes,
        vehicles,
    })
}
