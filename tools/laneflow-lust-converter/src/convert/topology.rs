//! Convert a parsed SUMO network into Traffic + Spatial packages.
//!
//! Emits Junction / Movement / ManeuverPath, optional static Signals,
//! vehicleProfiles, and optional DUE-derived routes + population table.

use crate::{
    Error, Result,
    convert::{
        junction::normalize_junctions,
        population::{
            POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS,
            POPULATION_DEPART_START_SECONDS, select_population,
        },
        routes::build_routes_and_bind_population,
        signals::convert_signals,
    },
    output::{
        TopologyArtifacts, compile_network_lfca, compile_network_lfca_with_infeasibility_report,
        geom::ReportSource,
        model::{
            Centerline, LaneEdge, LaneGraph, Parking, PopulationSelection, PopulationTableRecord,
            Route, RoutesToml, SpatialEdge, SpatialPackage, TrafficPackage, Units, VehicleProfile,
        },
    },
    sumo::{DueVehicle, LUST_FRAME_ID, SumoNetwork, SumoTlLogic},
};

pub(crate) const DEFAULT_FIXED_DELTA_MS: u64 = 16;
const DEFAULT_TRAFFIC_REF: &str = "lust-topology.traffic.json";
const DEFAULT_SPATIAL_REF: &str = "lust-topology.spatial.json";

/// Options for topology / static conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologyConvertOptions {
    pub fixed_delta_ms: u64,
    pub traffic_artifact_ref: String,
    pub spatial_artifact_ref: String,
    /// When true, require pinned LuST `<location>` lexical anchors.
    pub require_lust_location_anchors: bool,
    /// When true, require exactly 10,592 filtered DUE candidates before taking 10k.
    pub require_lust_population_count: bool,
    /// When true, deliver the deterministic emission-layer infeasibility
    /// diagnosis report instead of `network.lfca`（#253 验收重划，见 G1 补充
    /// 记录）；默认 false 保持 fail-fast 行为不变。
    pub emit_infeasibility_report: bool,
    /// stub 删焊授权域策略（#253 R8；默认 `Auto` = G1 授权域三条件）：
    /// `AllowUnrestricted` 仅限 fixture/单测，生产入口必须 Auto。
    pub stub_weld_policy: crate::convert::junction::StubWeldPolicy,
}

impl Default for TopologyConvertOptions {
    fn default() -> Self {
        Self {
            fixed_delta_ms: DEFAULT_FIXED_DELTA_MS,
            traffic_artifact_ref: DEFAULT_TRAFFIC_REF.to_owned(),
            spatial_artifact_ref: DEFAULT_SPATIAL_REF.to_owned(),
            require_lust_location_anchors: false,
            require_lust_population_count: false,
            emit_infeasibility_report: false,
            stub_weld_policy: crate::convert::junction::StubWeldPolicy::Auto,
        }
    }
}

/// Topology artifacts plus demand-side routes.toml bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticConversionArtifacts {
    pub topology: TopologyArtifacts,
    pub routes_toml: Vec<u8>,
    pub population_record_count: usize,
    pub route_count: usize,
}

/// Build validated Traffic/Spatial/Manifest bytes from a SUMO network.
pub fn convert_network_topology(
    network: &SumoNetwork,
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology_with_tll(network, &[], options)
}

/// ReportSource 缺省：结构化入口不持有原始字节，清单标注「未知」且不声称已校验。
fn unknown_report_source() -> ReportSource {
    ReportSource::unverified_unknown()
}

/// Build validated packages using static programs from `tll.static.xml`.
pub fn convert_network_topology_with_tll(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    options: &TopologyConvertOptions,
) -> Result<TopologyArtifacts> {
    convert_network_topology_with_tll_and_profiles(
        network,
        tll_programs,
        &[],
        options,
        unknown_report_source(),
    )
}

/// Build validated packages with static signals and vehicle profiles.
pub(crate) fn convert_network_topology_with_tll_and_profiles(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<TopologyArtifacts> {
    convert_network_packages(
        network,
        tll_programs,
        vehicle_profiles,
        &[],
        options,
        report_source,
    )
}

/// Convert topology + routes + population from network inputs and ordered DUE vehicles.
pub(crate) fn convert_static_with_due(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    due_vehicles: &[DueVehicle],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<StaticConversionArtifacts> {
    let topology_norm = normalize_junctions(network, &stub_weld_gate(options, &report_source))?;
    let population = select_population(due_vehicles, options.require_lust_population_count)?;
    let bundle = build_routes_and_bind_population(network, &topology_norm, &population)?;

    let topology = convert_network_packages(
        network,
        tll_programs,
        vehicle_profiles,
        &bundle.routes,
        options,
        report_source,
    )?;

    let table = RoutesToml {
        format_version: "0.1",
        selection: PopulationSelection {
            depart_start_seconds: POPULATION_DEPART_START_SECONDS,
            depart_end_seconds_exclusive: POPULATION_DEPART_END_SECONDS,
            require_lust_candidate_count: options.require_lust_population_count,
            candidate_count_expected: options
                .require_lust_population_count
                .then_some(POPULATION_CANDIDATE_COUNT as u64),
            selected_count: u64::try_from(bundle.records.len()).expect("count fits u64"),
            route_catalog_count: u64::try_from(bundle.routes.len()).expect("count fits u64"),
        },
        routes: bundle.routes.clone(),
        records: bundle
            .records
            .iter()
            .map(|record| PopulationTableRecord {
                population_rank: record.population_rank,
                vehicle_id: record.vehicle_id.clone(),
                vehicle_profile_id: record.vehicle_profile_id.clone(),
                depart_seconds: record.depart.to_string(),
                route_id: record.route_id.clone(),
                road_edge_ids: record.road_edge_ids.clone(),
                source_file_ordinal: record.source_file_ordinal,
                source_vehicle_ordinal: record.source_vehicle_ordinal,
            })
            .collect(),
    };
    let routes_toml = toml::to_string_pretty(&table).map_err(|source| Error::TomlSerialize {
        document: "routes.toml",
        source,
    })?;
    Ok(StaticConversionArtifacts {
        population_record_count: bundle.records.len(),
        route_count: bundle.routes.len(),
        topology,
        routes_toml: routes_toml.into_bytes(),
    })
}

/// R8 授权域门控推导：Auto 按 ReportSource 判定（verified 的类型级保证来自
/// R2——外部调用方无法构造 verified=true）；AllowUnrestricted 仅 fixture。
fn stub_weld_gate(
    options: &TopologyConvertOptions,
    report_source: &crate::output::geom::ReportSource,
) -> crate::convert::junction::StubWeldGate {
    use crate::convert::junction::{StubWeldGate, StubWeldPolicy};
    match options.stub_weld_policy {
        #[cfg(test)]
        StubWeldPolicy::AllowUnrestricted => StubWeldGate::Unrestricted,
        StubWeldPolicy::Auto => {
            if report_source.is_verified() {
                StubWeldGate::VerifiedDomain {
                    net_digest: report_source.net_digest().map(str::to_owned),
                }
            } else {
                StubWeldGate::BlockedDomain
            }
        }
    }
}

fn convert_network_packages(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    routes: &[Route],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<TopologyArtifacts> {
    if options.require_lust_location_anchors && !network.location.matches_lust_anchors() {
        return Err(Error::SumoModel(format!(
            "SUMO <location> does not match pinned LuST anchors (netOffset={:?}, convBoundary={:?})",
            network.location.net_offset_raw, network.location.conv_boundary_raw
        )));
    }

    let origin = network.location.canonical_origin()?;

    let topology = normalize_junctions(network, &stub_weld_gate(options, &report_source))?;
    let signals = convert_signals(network, tll_programs, &topology.path_by_connection)?;
    // G1 六条件之控制语义：焊接移除的 stub 内边不得出现在任何信号绑定
    // （stop line / maneuver gate 的路径 id）中；信号模型按运动（道路边）级
    // 绑定，本条为 fail-closed 守卫，语义漂移即拒。
    crate::convert::signals::validate_weld_signal_bindings(
        &signals,
        &topology.dropped_stub_lane_ids,
    )?;
    let stub_weld_records = topology.stub_weld_records.clone();

    let mut lane_edges = Vec::with_capacity(network.lanes.len());
    let mut spatial_edges = Vec::with_capacity(network.lanes.len());

    for lane in &network.lanes {
        // 点状 stub 内边已被 junction 归一化移除并焊接，不再进入 lane graph / spatial。
        if topology.dropped_stub_lane_ids.contains(lane.id.as_str()) {
            continue;
        }
        let id = lane.laneflow_id();
        let length = lane.length.to_f64()?;
        let speed_limit = lane.speed.to_f64()?;
        if length <= 0.0 {
            return Err(Error::SumoModel(format!(
                "lane {:?} length must be positive, got {length}",
                lane.id
            )));
        }
        if speed_limit <= 0.0 {
            return Err(Error::SumoModel(format!(
                "lane {:?} speed must be positive, got {speed_limit}",
                lane.id
            )));
        }
        // 编译器规定路口穿越由 ManeuverPath 独占权威：路径链不写 LaneEdge 后继
        // （涉及 internal 边的转移只能由 ManeuverPath 边序承载），因此 successor
        // 一律为空，车道连接关系由 junction 归一化（normalize_junctions）承担。
        lane_edges.push(LaneEdge {
            id: id.clone(),
            length,
            speed_limit,
        });

        let mut points = Vec::with_capacity(lane.shape.len());
        for (sx, sy) in &lane.shape {
            let projected_x = sx.checked_sub(network.location.net_offset.0)?;
            let projected_y = sy.checked_sub(network.location.net_offset.1)?;
            let x = projected_x.checked_sub(origin.0)?.to_f64()?;
            let z = projected_y.checked_sub(origin.1)?.to_f64()?;
            points.push([x, 0.0, z]);
        }
        // stub 焊接：入口边末点替换为出口边首点（同一投影链），消除点结处
        // 1–6 cm 的位置缝；被替换末点本就是 stub 起点的噪声坐标。
        if let Some((wx, wy)) = topology.stub_welds.get(lane.id.as_str()) {
            let projected_x = wx.checked_sub(network.location.net_offset.0)?;
            let projected_y = wy.checked_sub(network.location.net_offset.1)?;
            let x = projected_x.checked_sub(origin.0)?.to_f64()?;
            let z = projected_y.checked_sub(origin.1)?.to_f64()?;
            let last = points.last_mut().ok_or_else(|| {
                Error::SumoModel(format!("welded entry lane {:?} has empty shape", lane.id))
            })?;
            *last = [x, 0.0, z];
        }
        spatial_edges.push(SpatialEdge {
            traffic_edge_id: id,
            centerline: Centerline { points },
        });
    }

    let traffic = TrafficPackage {
        format_version: "0.8",
        units: Units {
            distance: "meter",
            time: "second",
        },
        lane_graph: LaneGraph { edges: lane_edges },
        junctions: topology.junctions,
        movements: topology.movements,
        maneuver_paths: topology.maneuver_paths,
        routes: routes.to_vec(),
        vehicle_profiles: vehicle_profiles.to_vec(),
        signals,
        parking: Parking {
            areas: Vec::new(),
            spaces: Vec::new(),
        },
        dropped_point_stub_edges: topology.dropped_stub_lane_ids.len() as u64,
    };
    let spatial = SpatialPackage {
        format_version: "0.1",
        frame_id: LUST_FRAME_ID.to_owned(),
        edges: spatial_edges,
    };

    if options.emit_infeasibility_report {
        compile_network_lfca_with_infeasibility_report(
            &traffic,
            &spatial,
            report_source,
            &stub_weld_records,
        )
    } else {
        compile_network_lfca(&traffic, &spatial)
    }
}
