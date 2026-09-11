//! 由拓扑编制复杂路口 catalog 并做交叉校验。

use laneflow_scenario::complex_junction::{
    AUTHORING_NAMESPACE, CATALOG_VERSION, JunctionCatalog, POLICY_KEY, PortalCatalogEntry,
    PortalLaneCatalogEntry, RouteCatalogEntry, SpawnSlotCatalogEntry,
    WeightedRouteChoiceCatalogEntry, validate,
};

use crate::Error;
use crate::config::JunctionConfig;
use crate::topology::TopologyBuild;

/// catalog 槽位总量上限：超出即配置错误，防止畸形配置（过长臂长 × 过小
/// pitch/车长）在编译限制介入前产出无界 catalog。
const TOTAL_SPAWN_SLOT_LIMIT: usize = 4_096;

/// 车辆横向包络的工程假定（profile 无 width 字段）；跨边槽位过滤用它区分
/// 「相邻车道并排」（安全）与「近乎同线前后」（需间隔一个车长）。
const CAR_WIDTH_METERS: f64 = 2.0;

pub(crate) fn build_catalog(
    config: &JunctionConfig,
    topology: &TopologyBuild,
) -> Result<JunctionCatalog, Error> {
    let clearance = config.endpoint_clearance_meters();
    let vehicle_length = config.profile.length_meters;
    let mut slots = Vec::new();
    // 已放置槽位的折线采样位置与单位切向；跨边候选槽位逐一与之做重叠检查
    // （横向 < 车宽包络且纵向 < 车长即跳过），同角环路在分叉/汇合段几何
    // 收敛处靠该贪心过滤保证不重叠，而非依赖恒定进度相位。
    let mut placed: Vec<(crate::topology::Point, crate::topology::Point)> = Vec::new();
    let mut portals = Vec::new();
    for portal in &topology.portals {
        let mut lanes = Vec::new();
        for (lane_index, lane) in portal.lanes.iter().enumerate() {
            let curve = &topology.edge(&lane.edge).expect("portal lane edge").curve;
            let length = curve.chord_length();
            // lane 1 的候选从半个间距处起错，提升交错密度；不安全者被过滤。
            let phase = lane_index as f64 * config.geometry.spawn_slot_pitch_meters / 2.0;
            let minimum_length = clearance * 2.0 + phase;
            if length < minimum_length {
                return Err(Error::Config(format!(
                    "portal {:?} lane {lane_index} loop edge {:?} is {length} m long; \
                     at least {minimum_length} m is required for one spawn slot",
                    portal.id, lane.edge
                )));
            }
            let mut entry_spawn_slot_id = None;
            let mut local_index = 0_u32;
            let mut progress = clearance + phase;
            while progress <= length - clearance {
                let sample = curve
                    .point_at_chord(progress)
                    .expect("slot progress within chord length");
                let (point, tangent) = sample;
                let safe = placed.iter().all(|&(other_point, _)| {
                    let delta = [other_point[0] - point[0], other_point[1] - point[1]];
                    let lateral = (delta[0] * tangent[1] - delta[1] * tangent[0]).abs();
                    let along = (delta[0] * tangent[0] + delta[1] * tangent[1]).abs();
                    lateral >= CAR_WIDTH_METERS || along >= vehicle_length
                });
                if safe {
                    let slot_id = format!("slot-{}-{local_index:03}", lane.edge);
                    entry_spawn_slot_id.get_or_insert_with(|| slot_id.clone());
                    slots.push(SpawnSlotCatalogEntry {
                        slot_id,
                        portal_id: portal.id.to_owned(),
                        lane_index,
                        edge_id: lane.edge.clone(),
                        progress,
                    });
                    placed.push(sample);
                    if slots.len() > TOTAL_SPAWN_SLOT_LIMIT {
                        return Err(Error::Config(format!(
                            "configuration yields more than {TOTAL_SPAWN_SLOT_LIMIT} spawn \
                             slots; reduce arm_length_meters or increase spawn_slot_pitch_meters"
                        )));
                    }
                }
                local_index += 1;
                progress += config.geometry.spawn_slot_pitch_meters;
            }
            let Some(entry_spawn_slot_id) = entry_spawn_slot_id else {
                return Err(Error::Config(format!(
                    "portal {:?} lane {lane_index} loop edge {:?} has no non-overlapping \
                     spawn slot; increase loop separation or widen the pitch",
                    portal.id, lane.edge
                )));
            };
            lanes.push(PortalLaneCatalogEntry {
                lane_index,
                entry_spawn_slot_id,
                route_choices: lane
                    .choices
                    .iter()
                    .map(|&(route_id, weight)| WeightedRouteChoiceCatalogEntry {
                        route_id: route_id.to_owned(),
                        weight,
                    })
                    .collect(),
            });
        }
        portals.push(PortalCatalogEntry {
            id: portal.id.to_owned(),
            lanes,
        });
    }
    let routes = topology
        .routes
        .iter()
        .map(|route| RouteCatalogEntry {
            route_id: route.id.to_owned(),
            exit_portal_id: route.exit_portal.to_owned(),
            edge_ids: route.edges.clone(),
            focus: route.focus,
        })
        .collect();

    Ok(JunctionCatalog {
        catalog_version: CATALOG_VERSION.to_owned(),
        policy_selection: laneflow_scenario::complex_junction::CatalogPolicySelection::Pinned {
            policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                    AUTHORING_NAMESPACE,
                    POLICY_KEY,
                    &laneflow_compiler::CompileLimits::p100_initial_v1(),
                )
                .map_err(|error| Error::Catalog(format!("policy identity: {error:?}")))?,
            )
            .to_string(),
        },
        portals,
        routes,
        spawn_slots: slots,
    })
}

pub(crate) fn validate_catalog(
    catalog: &JunctionCatalog,
    topology: &TopologyBuild,
    config: &JunctionConfig,
) -> Result<(), Error> {
    let encoded = toml::to_string(catalog)?;
    let decoded: JunctionCatalog =
        toml::from_str(&encoded).map_err(|error| Error::Catalog(error.to_string()))?;
    if decoded != *catalog {
        return Err(Error::Catalog(
            "TOML round trip changed catalog semantics".to_owned(),
        ));
    }
    validate(catalog).map_err(|error| Error::Catalog(error.to_string()))?;

    let route_by_id = topology
        .routes
        .iter()
        .map(|route| (route.id, route))
        .collect::<std::collections::HashMap<_, _>>();
    let lane_by_key = catalog
        .portals
        .iter()
        .flat_map(|portal| {
            portal
                .lanes
                .iter()
                .map(move |lane| ((portal.id.as_str(), lane.lane_index), lane))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let clearance = config.endpoint_clearance_meters();
    for slot in &catalog.spawn_slots {
        let lane = lane_by_key
            .get(&(slot.portal_id.as_str(), slot.lane_index))
            .expect("scenario validate checked portal lane");
        for choice in &lane.route_choices {
            let route = route_by_id
                .get(choice.route_id.as_str())
                .ok_or_else(|| Error::Catalog(format!("unknown route {:?}", choice.route_id)))?;
            if route.edges.first().map(String::as_str) != Some(slot.edge_id.as_str()) {
                return Err(Error::Catalog(format!(
                    "slot {:?} edge_id is not the entry edge of route {:?}",
                    slot.slot_id, choice.route_id
                )));
            }
        }
        // slot progress 闭区间对照 corridor-generator：端点净距内不许有停车位。
        let length = topology.edge_length(&slot.edge_id);
        if !slot.progress.is_finite()
            || slot.progress < clearance
            || slot.progress > length - clearance
        {
            return Err(Error::Catalog(format!(
                "slot {:?} progress {} outside [{clearance}, {}]",
                slot.slot_id,
                slot.progress,
                length - clearance
            )));
        }
    }
    Ok(())
}
