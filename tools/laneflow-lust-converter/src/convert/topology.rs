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
        profiles::{convert_vehicle_profiles, select_passenger_vtypes},
        routes::{build_routes_and_bind_population, validate_population_route_edges},
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
    sumo::{
        DueVehicle, LUST_FRAME_ID, SumoNetwork, SumoTlLogic, parse_due_routes_xml,
        parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml,
    },
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
    /// Topology artifacts（诊断清单或编译产物）。
    pub topology: TopologyArtifacts,
    /// DUE route catalog + population table；诊断模式（#253 L1 绕过未完成的
    /// lane-level 展开）为 None——不产出 routes.toml。
    pub routes_toml: Option<Vec<u8>>,
    pub population_record_count: usize,
    pub route_count: usize,
    /// 「声明臂无受控 link」source-health 事实（#253 K4a；通常为 empty）。
    pub signal_health: Vec<crate::convert::signals::UnclaimedSignalArm>,
}

/// Build validated packages with static signals and vehicle profiles.
///
/// 当前仅验收套件消费（无 tll/profiles 的 topology-only 组合在公开面收敛后
/// 没有生产调用方）；生产路径是 [`convert_static_with_due`]。新增生产调用方
/// 时去掉 `cfg(test)` 即可。
#[cfg(test)]
pub(crate) fn convert_network_topology_with_tll_and_profiles(
    network: &SumoNetwork,
    tll_programs: &[SumoTlLogic],
    vehicle_profiles: &[VehicleProfile],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<TopologyArtifacts> {
    // signal health 事实只经 static 入口（conversion report）携带；topology-only
    // 路径（诊断清单交付）不产 report，丢弃。
    let (artifacts, _) = convert_network_packages(
        network,
        tll_programs,
        vehicle_profiles,
        &[],
        options,
        report_source,
    )?;
    Ok(artifacts)
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
    // population 选取与健康事实（精确 10,000 计数）两模式都保留。
    let population = select_population(due_vehicles, options.require_lust_population_count)?;

    // #253 L1：诊断模式绕过未完成的 lane-level route 展开（`can_complete`
    // 禁止边内换道，pinned 入选 10,000 中 9,350 条不可展开——实现语义缺口，
    // 另立 issue 修复；§4/§3.6 文档已标「routes.toml 实现中」）。不展开即不产
    // routes.toml（None）；fail-fast 路径保持原语义（展开失败仍 fail-closed）。
    // #253 W2：跳过的是展开，不是入选集校验——诊断模式仍执行 route 边引用
    // 存在性/非 internal 校验（K3(a) 失败域不因模式而豁免）。
    let (routes, routes_toml, population_record_count) = if options.emit_infeasibility_report {
        validate_population_route_edges(network, &population)?;
        (Vec::new(), None, population.len())
    } else {
        // 归一化只在 fail-fast 的 route 展开分支需要——诊断模式的结果由
        // convert_network_packages 内部归一化产出，此处不再重复计算。
        let topology_norm = normalize_junctions(network, &stub_weld_gate(options, &report_source))?;
        let bundle = build_routes_and_bind_population(network, &topology_norm, &population)?;
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
        let routes_toml =
            toml::to_string_pretty(&table).map_err(|source| Error::TomlSerialize {
                document: "routes.toml",
                source,
            })?;
        (
            bundle.routes,
            Some(routes_toml.into_bytes()),
            bundle.records.len(),
        )
    };

    let (topology, signal_health) = convert_network_packages(
        network,
        tll_programs,
        vehicle_profiles,
        &routes,
        options,
        report_source,
    )?;

    Ok(StaticConversionArtifacts {
        population_record_count,
        route_count: routes.len(),
        topology,
        routes_toml,
        signal_health,
    })
}

/// Convert topology + DUE routes + population table from net/tll/vtypes/DUE XML.
///
/// `due_xmls` must be the three `local.static.{0,1,2}.rou.xml` texts in order.
/// 生产组合入口：crate 内 pipeline（`output::pipeline::convert_verified`）调用。
pub(crate) fn convert_static_from_xml_with_due_and_source(
    net_xml: &str,
    tll_xml: &str,
    vtypes_xml: &str,
    due_xmls: [&str; 3],
    options: &TopologyConvertOptions,
    report_source: ReportSource,
) -> Result<StaticConversionArtifacts> {
    let network = parse_sumo_network_xml(net_xml)?;
    let tll = parse_tll_static_xml(tll_xml)?;
    let vtypes = parse_vtypes_xml(vtypes_xml)?;
    let passengers = select_passenger_vtypes(&vtypes)?;
    let profiles = convert_vehicle_profiles(&passengers)?;
    let mut due_vehicles = Vec::new();
    for (ordinal, xml) in due_xmls.into_iter().enumerate() {
        let file_ordinal = u8::try_from(ordinal).expect("0..2 fits u8");
        due_vehicles.extend(parse_due_routes_xml(xml, file_ordinal)?);
    }
    convert_static_with_due(
        &network,
        &tll,
        &profiles,
        &due_vehicles,
        options,
        report_source,
    )
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
) -> Result<(
    TopologyArtifacts,
    Vec<crate::convert::signals::UnclaimedSignalArm>,
)> {
    if options.require_lust_location_anchors && !network.location.matches_lust_anchors() {
        return Err(Error::SumoModel(format!(
            "SUMO <location> does not match pinned LuST anchors (netOffset={:?}, convBoundary={:?})",
            network.location.net_offset_raw, network.location.conv_boundary_raw
        )));
    }

    let origin = network.location.canonical_origin()?;

    let topology = normalize_junctions(network, &stub_weld_gate(options, &report_source))?;
    let (signals, signal_health) =
        convert_signals(network, tll_programs, &topology.path_by_connection)?;
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

    // #253 Q6：诊断条目的 internal/authored 分类按权威 function 元数据
    // （P4 已证 pinned 两法 0 不一致，survey 字节不变）。
    let internal_lanes: std::collections::HashSet<String> = network
        .lanes
        .iter()
        .filter(|lane| lane.function_internal)
        .map(|lane| lane.laneflow_id())
        .collect();
    let artifacts = if options.emit_infeasibility_report {
        compile_network_lfca_with_infeasibility_report(
            &traffic,
            &spatial,
            report_source,
            &stub_weld_records,
            &internal_lanes,
        )
    } else {
        compile_network_lfca(&traffic, &spatial, &internal_lanes)
    }?;
    Ok((artifacts, signal_health))
}

#[cfg(test)]
mod policy_gate_tests {
    use super::*;
    use crate::convert::junction::{StubWeldGate, StubWeldPolicy};

    /// R8 测试策略的显式逃生口：`AllowUnrestricted`（仅限 crate 内单测）经
    /// 门控推导必须得到 `Unrestricted`——否则该变体沦为不可达的摆设，且
    /// fixture 只能绕过策略直接伪造 gate，丧失「策略→门控」链路的覆盖。
    #[test]
    fn allow_unrestricted_policy_derives_unrestricted_gate() {
        let options = TopologyConvertOptions {
            stub_weld_policy: StubWeldPolicy::AllowUnrestricted,
            ..TopologyConvertOptions::default()
        };
        let gate = stub_weld_gate(&options, &ReportSource::unverified_unknown());
        assert!(
            matches!(gate, StubWeldGate::Unrestricted),
            "AllowUnrestricted must derive the Unrestricted gate"
        );
    }

    /// 对称锚点：默认 Auto + 未 verified 来源必须保持 BlockedDomain（R8
    /// 授权域三条件缺一即拒）。
    #[test]
    fn auto_policy_with_unverified_source_stays_blocked() {
        let gate = stub_weld_gate(
            &TopologyConvertOptions::default(),
            &ReportSource::unverified_unknown(),
        );
        assert!(
            matches!(gate, StubWeldGate::BlockedDomain),
            "Auto + unverified must stay BlockedDomain"
        );
    }

    /// #253 W2：诊断模式跳过 lane 级展开，但不跳过入选集 route 边引用校验——
    /// unknown/internal 边引用在诊断模式下同样 fail-closed（K3(a) 失败域不
    /// 因模式而豁免）。
    #[test]
    fn diagnostic_mode_rejects_bad_population_edge_references() {
        let net = include_str!("../../tests/fixtures/minimal/t-junction.net.xml");
        let tll = include_str!("../../tests/fixtures/minimal/t-junction.tll.xml");
        let vtypes = include_str!("../../tests/fixtures/minimal/vtypes.add.xml");
        let options = TopologyConvertOptions {
            emit_infeasibility_report: true,
            ..TopologyConvertOptions::default()
        };
        let due_unknown = r#"<routes>
  <vehicle id="v0" type="passenger1" depart="28800" departPos="random">
    <route edges="west ghost"/>
  </vehicle>
</routes>"#;
        let error = super::convert_static_from_xml_with_due_and_source(
            net,
            tll,
            vtypes,
            [due_unknown, "<routes/>", "<routes/>"],
            &options,
            ReportSource::unverified_unknown(),
        )
        .expect_err("diagnostic mode must reject unknown route edges");
        assert!(error.to_string().contains("unknown road edge"), "{error}");

        let due_internal = r#"<routes>
  <vehicle id="v1" type="passenger1" depart="28800" departPos="random">
    <route edges="west :J_0"/>
  </vehicle>
</routes>"#;
        let error = super::convert_static_from_xml_with_due_and_source(
            net,
            tll,
            vtypes,
            [due_internal, "<routes/>", "<routes/>"],
            &options,
            ReportSource::unverified_unknown(),
        )
        .expect_err("diagnostic mode must reject internal route edges");
        assert!(error.to_string().contains("internal edge"), "{error}");
    }
}
