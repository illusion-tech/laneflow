use laneflow_compiler::{CompileLimits, derive_canonical_stable_id_v1};
use laneflow_static_contract::EntityKind;
use serde::{Deserialize, Serialize};

use crate::{Case, Error, Identity, NAMESPACE, POLICY_KEY, compile_error, key};

/// 由稳定标识引用静态实体的初始状态目录。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InitialState {
    pub format: String,
    pub identity: Identity,
    pub policy_id: String,
    pub vehicle_profile_id: String,
    pub routes: Vec<RouteRecord>,
    pub vehicles: Vec<VehicleRecord>,
}

/// 路线的两个有向 edge；数组顺序就是 occurrence 顺序。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteRecord {
    pub route_rank: u32,
    pub cell: u32,
    pub route: u32,
    pub edge_ids: [String; 2],
    pub signal_group_id: String,
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
    pub parking_space_id: String,
}

pub(crate) fn stable_id(kind: EntityKind, source_key: &str) -> Result<String, Error> {
    let id = derive_canonical_stable_id_v1(
        kind,
        NAMESPACE,
        source_key,
        &CompileLimits::single_network_1m_v2(),
    )
    .map_err(|error| compile_error("stable identity", error))?;
    Ok(format!("lfid1_{}_{id:x}", kind.slug()))
}

pub(crate) fn initial_state(identity: &Identity) -> Result<InitialState, Error> {
    let mut routes = Vec::new();
    let mut vehicles = Vec::new();
    for cell in 0..identity.cells() {
        for route in 0..4 {
            let route_rank = cell * 4 + route;
            routes.push(RouteRecord {
                route_rank,
                cell,
                route,
                edge_ids: [
                    stable_id(EntityKind::LaneEdge, &key(cell, route, "entry"))?,
                    stable_id(EntityKind::LaneEdge, &key(cell, route, "exit"))?,
                ],
                signal_group_id: stable_id(EntityKind::SignalGroup, &key(cell, route, "group"))?,
            });
            for slot in 0..25 {
                let active = match identity.case {
                    Case::W1 | Case::W4 => route < 2 || slot < if route == 2 { 12 } else { 13 },
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
                    parking_space_id: stable_id(
                        EntityKind::ParkingSpace,
                        &key(cell, route, &format!("space{slot:02}")),
                    )?,
                });
            }
        }
    }
    Ok(InitialState {
        format: "lf-synth-state-1".into(),
        identity: identity.clone(),
        policy_id: stable_id(EntityKind::RightOfWayPolicySet, POLICY_KEY)?,
        vehicle_profile_id: stable_id(EntityKind::VehicleProfile, "car")?,
        routes,
        vehicles,
    })
}
