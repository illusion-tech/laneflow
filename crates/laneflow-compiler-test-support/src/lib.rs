#![doc = include_str!("../README.md")]

use std::{error::Error, fmt};

use laneflow_compiler::{
    CanonicalAccessTarget, CanonicalCorridorElement, CanonicalSignalControl, ValidatedCanonicalLir,
};
use laneflow_core::{
    AccessEffect, AccessRegistry, AccessRule, AccessTargetId, CorridorElementId,
    CrossSectionRegistry, EdgeLength, FacilityBand, IidmProfileSpec, InitialTrafficData, Junction,
    JunctionRegistry, LaneEdge, LaneGraph, LaneGroup, ManeuverGate, ManeuverPath, Movement,
    ParkingArea, ParkingRegistry, ParkingSpace, ParkingSpaceGeometry, ParticipantClass,
    ParticipantClassRegistry, RoadCorridor, RoadSection, Route, SectionLane, SignalAspect,
    SignalControlInput, SignalController, SignalGroup, SignalGroupState, SignalPhase,
    SignalRegistry, SpeedLimit, StopLine, StopLineLocation, VehicleProfile, VehicleProfileRegistry,
    WaitingRegistry, WaitingZone,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, SpatialEdgeInput, SpatialError, SpatialRegistry,
};
use laneflow_static_contract::{EntityKind, EntityKindMarker, StableId, StableId128};

/// 一条 Canonical LIR 实体到当前态定位键的稳定对应关系。
///
/// `lir_ordinal` 只在产生本报告的 LIR 内有效；跨修订比较必须使用 `stable_id`。
/// 除 `AuthoringLane` 外，当前投影把规范稳定标识文本直接用作 external ID。当前
/// `SectionLane` 没有独立 ID，因此其 `current_external_id` 为 `None`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionMapping {
    entity_kind: EntityKind,
    lir_ordinal: u32,
    stable_id: StableId128,
    current_external_id: Option<Box<str>>,
}

impl ProjectionMapping {
    /// 返回 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(&self) -> EntityKind {
        self.entity_kind
    }

    /// 返回实体在本次 LIR 表中的致密序号。
    #[must_use]
    pub const fn lir_ordinal(&self) -> u32 {
        self.lir_ordinal
    }

    /// 返回可跨当前对象图与 LIR 关联的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> StableId128 {
        self.stable_id
    }

    /// 返回投影到当前对象图的 external ID；没有独立 ID 的嵌入记录返回 `None`。
    #[must_use]
    pub fn current_external_id(&self) -> Option<&str> {
        self.current_external_id.as_deref()
    }
}

/// 按 `EntityKind` 登记顺序、再按 LIR ordinal 排列的完整投影映射报告。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionMappingReport {
    entries: Box<[ProjectionMapping]>,
}

impl ProjectionMappingReport {
    /// 返回全部映射项。
    #[must_use]
    pub const fn entries(&self) -> &[ProjectionMapping] {
        &self.entries
    }

    /// 返回指定实体种类的连续映射项。
    pub fn entries_for(&self, entity_kind: EntityKind) -> impl Iterator<Item = &ProjectionMapping> {
        self.entries
            .iter()
            .filter(move |entry| entry.entity_kind == entity_kind)
    }
}

/// 一次完整的当前态集成投影结果。
///
/// `traffic` 拥有全部当前 Core 静态登记表；`spatial` 仅在 LIR 带完整规范几何时存在。
/// 当前 `SpatialRegistry` 只能完整绑定一个 frame，因此多个互不连通 frame 的合法 LIR
/// 会由投影边界显式拒绝，而不会丢弃或重解释 frame 身份。
pub struct CurrentProjection {
    traffic: InitialTrafficData,
    spatial: Option<SpatialRegistry>,
    mappings: ProjectionMappingReport,
}

impl CurrentProjection {
    /// 返回已通过当前 Core 构造器校验的初始交通数据。
    #[must_use]
    pub const fn traffic(&self) -> &InitialTrafficData {
        &self.traffic
    }

    /// 返回完整绑定的当前空间登记表；headless LIR 返回 `None`。
    #[must_use]
    pub const fn spatial(&self) -> Option<&SpatialRegistry> {
        self.spatial.as_ref()
    }

    /// 返回用于端到端等价断言的稳定映射报告。
    #[must_use]
    pub const fn mappings(&self) -> &ProjectionMappingReport {
        &self.mappings
    }

    /// 拆分投影结果；各部分仍只用于迁移验证，不构成生产加载接口。
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        InitialTrafficData,
        Option<SpatialRegistry>,
        ProjectionMappingReport,
    ) {
        (self.traffic, self.spatial, self.mappings)
    }
}

/// 当前态投影失败。
#[derive(Debug)]
pub enum ProjectionError {
    /// LIR 几何表只覆盖部分车道图边；合法编译结果不应出现该状态。
    PartialSpatialCoverage {
        /// LaneEdge 总数。
        lane_edge_count: usize,
        /// 带规范几何的 LaneEdge 数量。
        geometry_count: usize,
    },
    /// 当前单个 `SpatialRegistry` 无法表达多个规范坐标框架。
    MultipleCanonicalFrames,
    /// 当前 Core 构造器拒绝了投影对象。
    Core(Box<laneflow_core::CoreError>),
    /// 当前 Spatial 构造器拒绝了投影对象。
    Spatial(Box<SpatialError>),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PartialSpatialCoverage {
                lane_edge_count,
                geometry_count,
            } => write!(
                formatter,
                "Canonical LIR 空间几何覆盖不完整：LaneEdge {lane_edge_count} 条，几何 {geometry_count} 条"
            ),
            Self::MultipleCanonicalFrames => formatter.write_str(
                "当前 SpatialRegistry 只能完整绑定一个 CanonicalFrame，不能投影多 frame LIR",
            ),
            Self::Core(source) => write!(formatter, "当前 Core 构造拒绝投影：{source}"),
            Self::Spatial(source) => write!(formatter, "当前 Spatial 构造拒绝投影：{source}"),
        }
    }
}

impl Error for ProjectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Core(source) => Some(source.as_ref()),
            Self::Spatial(source) => Some(source.as_ref()),
            Self::PartialSpatialCoverage { .. } | Self::MultipleCanonicalFrames => None,
        }
    }
}

impl From<laneflow_core::CoreError> for ProjectionError {
    fn from(source: laneflow_core::CoreError) -> Self {
        Self::Core(Box::new(source))
    }
}

impl From<SpatialError> for ProjectionError {
    fn from(source: SpatialError) -> Self {
        Self::Spatial(Box::new(source))
    }
}

/// 把已验证 Canonical LIR 投影为当前 Core/Spatial 对象图。
///
/// 本函数不接受 AST、HIR、MIR、JSON 或未验证表。所有关系均通过 LIR ordinal 翻译，
/// 不从几何、字符串或当前对象图反向推断编译器语义。当前构造器仍执行自己的防御校验，
/// 因而失败会作为 [`ProjectionError`] 返回且不发布部分结果。
///
/// # Errors
///
/// 当前 Core/Spatial 构造器拒绝投影，空间几何覆盖不完整，或合法 LIR 使用多个
/// canonical frame 而当前 `SpatialRegistry` 无法表达时返回错误。
pub fn project(lir: &ValidatedCanonicalLir) -> Result<CurrentProjection, ProjectionError> {
    let ids = ProjectionIds::from_lir(lir);

    let lane_graph = project_lane_graph(lir, &ids)?;
    let cross_section = project_cross_section(lir, &ids, &lane_graph)?;
    let junctions = project_junctions(lir, &ids, &lane_graph)?;
    let signals = project_signals(lir, &ids, &lane_graph, &junctions)?;
    let waiting = project_waiting(lir, &ids, &junctions, &signals)?;
    let parking = project_parking(lir, &ids, &lane_graph)?;
    let participant_classes = project_participant_classes(lir, &ids)?;
    let vehicle_profiles = project_vehicle_profiles(lir, &ids, &participant_classes)?;
    let access = project_access(
        lir,
        &ids,
        &lane_graph,
        &junctions,
        &cross_section,
        &participant_classes,
    )?;
    let routes = project_routes(lir, &ids)?;

    let traffic = InitialTrafficData::try_new_with_waiting(
        lane_graph,
        routes,
        vehicle_profiles,
        junctions,
        signals,
        parking,
        participant_classes,
        cross_section,
        access,
        waiting,
    )?;
    let spatial = project_spatial(lir, &ids, traffic.lane_graph())?;
    let mappings = ProjectionMappingReport {
        entries: projection_mappings(lir, &ids).into_boxed_slice(),
    };

    Ok(CurrentProjection {
        traffic,
        spatial,
        mappings,
    })
}

struct ProjectionIds {
    lane_edges: Vec<String>,
    road_corridors: Vec<String>,
    road_sections: Vec<String>,
    lane_groups: Vec<String>,
    facility_bands: Vec<String>,
    junctions: Vec<String>,
    movements: Vec<String>,
    maneuver_paths: Vec<String>,
    stop_lines: Vec<String>,
    maneuver_gates: Vec<String>,
    waiting_zones: Vec<String>,
    signal_groups: Vec<String>,
    signal_controllers: Vec<String>,
    signal_phases: Vec<String>,
    parking_areas: Vec<String>,
    parking_spaces: Vec<String>,
    participant_classes: Vec<String>,
    vehicle_profiles: Vec<String>,
    canonical_frames: Vec<String>,
    access_rules: Vec<String>,
    static_routes: Vec<String>,
}

impl ProjectionIds {
    fn from_lir(lir: &ValidatedCanonicalLir) -> Self {
        Self {
            lane_edges: lir
                .lane_edges()
                .map(|view| view.stable_id().to_string())
                .collect(),
            road_corridors: lir
                .road_corridors()
                .map(|view| view.stable_id().to_string())
                .collect(),
            road_sections: lir
                .road_sections()
                .map(|view| view.stable_id().to_string())
                .collect(),
            lane_groups: lir
                .lane_groups()
                .map(|view| view.stable_id().to_string())
                .collect(),
            facility_bands: lir
                .facility_bands()
                .map(|view| view.stable_id().to_string())
                .collect(),
            junctions: lir
                .junctions()
                .map(|view| view.stable_id().to_string())
                .collect(),
            movements: lir
                .movements()
                .map(|view| view.stable_id().to_string())
                .collect(),
            maneuver_paths: lir
                .maneuver_paths()
                .map(|view| view.stable_id().to_string())
                .collect(),
            stop_lines: lir
                .stop_lines()
                .map(|view| view.stable_id().to_string())
                .collect(),
            maneuver_gates: lir
                .maneuver_gates()
                .map(|view| view.stable_id().to_string())
                .collect(),
            waiting_zones: lir
                .waiting_zones()
                .map(|view| view.stable_id().to_string())
                .collect(),
            signal_groups: lir
                .signal_groups()
                .map(|view| view.stable_id().to_string())
                .collect(),
            signal_controllers: lir
                .signal_controllers()
                .map(|view| view.stable_id().to_string())
                .collect(),
            signal_phases: lir
                .signal_phases()
                .map(|view| view.stable_id().to_string())
                .collect(),
            parking_areas: lir
                .parking_areas()
                .map(|view| view.stable_id().to_string())
                .collect(),
            parking_spaces: lir
                .parking_spaces()
                .map(|view| view.stable_id().to_string())
                .collect(),
            participant_classes: lir
                .participant_classes()
                .map(|view| view.stable_id().to_string())
                .collect(),
            vehicle_profiles: lir
                .vehicle_profiles()
                .map(|view| view.stable_id().to_string())
                .collect(),
            canonical_frames: lir
                .canonical_frames()
                .map(|view| view.stable_id().to_string())
                .collect(),
            access_rules: lir
                .access_rules()
                .map(|view| view.stable_id().to_string())
                .collect(),
            static_routes: lir
                .static_routes()
                .map(|view| view.stable_id().to_string())
                .collect(),
        }
    }
}

fn project_lane_graph(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
) -> Result<LaneGraph, ProjectionError> {
    let edges = lir
        .lane_edges()
        .map(|edge| -> Result<_, ProjectionError> {
            let successors = edge
                .successors()
                .iter()
                .map(|ordinal| ids.lane_edges[ordinal.index()].clone());
            Ok(LaneEdge::new(
                ids.lane_edges[edge.ordinal().index()].clone(),
                EdgeLength::try_new(edge.length_meters())?,
                SpeedLimit::try_new(edge.speed_limit_meters_per_second())?,
                successors,
            ))
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    Ok(LaneGraph::try_new(edges)?)
}

fn project_cross_section(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
) -> Result<CrossSectionRegistry, ProjectionError> {
    let bands = lir.facility_bands().map(|band| {
        FacilityBand::new(
            ids.facility_bands[band.ordinal().index()].clone(),
            band.kind_id(),
        )
    });
    let sections = lir.road_sections().map(|section| {
        let lanes = section.lanes().iter().map(|ordinal| {
            let lane = lir
                .authoring_lane(*ordinal)
                .expect("Validated LIR authoring lane ordinal must resolve");
            let edge_ids = lane
                .edge_chain()
                .iter()
                .map(|edge| ids.lane_edges[edge.index()].clone());
            let group = lane
                .lane_group()
                .map(|group| ids.lane_groups[group.index()].as_str());
            SectionLane::new(edge_ids, group)
        });
        RoadSection::new(
            ids.road_sections[section.ordinal().index()].clone(),
            section.kind_id(),
            lanes,
        )
    });
    let groups = lir.lane_groups().map(|group| {
        LaneGroup::new(
            ids.lane_groups[group.ordinal().index()].clone(),
            ids.road_sections[group.road_section().index()].clone(),
        )
    });
    let corridors = lir.road_corridors().map(|corridor| {
        let elements = corridor.elements().map(|element| match element {
            CanonicalCorridorElement::RoadSection(section) => {
                CorridorElementId::Section(ids.road_sections[section.index()].clone())
            }
            CanonicalCorridorElement::FacilityBand(band) => {
                CorridorElementId::Band(ids.facility_bands[band.index()].clone())
            }
            _ => {
                unreachable!("compiler and integration projection use the same closed LIR version")
            }
        });
        RoadCorridor::new(
            ids.road_corridors[corridor.ordinal().index()].clone(),
            ids.road_sections[corridor.reference_section().index()].clone(),
            elements,
        )
    });
    Ok(CrossSectionRegistry::try_new(
        lane_graph, bands, sections, groups, corridors,
    )?)
}

fn project_junctions(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
) -> Result<JunctionRegistry, ProjectionError> {
    let junctions = lir
        .junctions()
        .map(|view| Junction::new(ids.junctions[view.ordinal().index()].clone()));
    let movements = lir.movements().map(|view| {
        Movement::new(
            ids.movements[view.ordinal().index()].clone(),
            ids.junctions[view.junction().index()].clone(),
        )
    });
    let paths = lir.maneuver_paths().map(|view| {
        ManeuverPath::new(
            ids.maneuver_paths[view.ordinal().index()].clone(),
            ids.movements[view.movement().index()].clone(),
            ids.lane_edges[view.entry_edge().index()].clone(),
            view.internal_edges()
                .iter()
                .map(|edge| ids.lane_edges[edge.index()].clone()),
            ids.lane_edges[view.exit_edge().index()].clone(),
        )
    });
    Ok(JunctionRegistry::try_new(
        lane_graph, junctions, movements, paths,
    )?)
}

fn project_signals(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
    junctions: &JunctionRegistry,
) -> Result<SignalRegistry, ProjectionError> {
    let stop_lines = lir.stop_lines().map(|view| {
        StopLine::new(
            ids.stop_lines[view.ordinal().index()].clone(),
            ids.lane_edges[view.lane_edge().index()].clone(),
            StopLineLocation::EdgeEnd,
        )
    });
    let groups = lir
        .signal_groups()
        .map(|view| SignalGroup::new(ids.signal_groups[view.ordinal().index()].clone()));
    let controllers = lir.signal_controllers().map(|controller| {
        let phases = controller.phases().iter().map(|ordinal| {
            let phase = lir
                .signal_phase(*ordinal)
                .expect("Validated LIR signal phase ordinal must resolve");
            let states = phase.states().map(|state| {
                SignalGroupState::new(
                    ids.signal_groups[state.signal_group().index()].clone(),
                    match state.aspect() {
                        laneflow_compiler::SignalAspect::Red => SignalAspect::Red,
                        laneflow_compiler::SignalAspect::Yellow => SignalAspect::Yellow,
                        laneflow_compiler::SignalAspect::Green => SignalAspect::Green,
                        _ => unreachable!(
                            "compiler and integration projection use the same closed LIR version"
                        ),
                    },
                )
            });
            SignalPhase::new(
                ids.signal_phases[phase.ordinal().index()].clone(),
                phase.duration_ms(),
                states,
            )
        });
        SignalController::new_fixed_time(
            ids.signal_controllers[controller.ordinal().index()].clone(),
            controller.offset_ms(),
            controller
                .signal_groups()
                .iter()
                .map(|group| ids.signal_groups[group.index()].clone()),
            phases,
        )
    });
    let gates = lir.maneuver_gates().map(|view| {
        let control = match view.signal_control() {
            CanonicalSignalControl::Group(group) => {
                SignalControlInput::Group(ids.signal_groups[group.index()].clone())
            }
            CanonicalSignalControl::None => SignalControlInput::None,
            _ => {
                unreachable!("compiler and integration projection use the same closed LIR version")
            }
        };
        ManeuverGate::new(
            ids.maneuver_gates[view.ordinal().index()].clone(),
            ids.maneuver_paths[view.maneuver_path().index()].clone(),
            view.transition_index(),
            ids.stop_lines[view.stop_line().index()].clone(),
            control,
        )
    });
    Ok(SignalRegistry::try_new(
        lane_graph,
        junctions,
        stop_lines,
        groups,
        controllers,
        gates,
    )?)
}

fn project_waiting(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    junctions: &JunctionRegistry,
    signals: &SignalRegistry,
) -> Result<WaitingRegistry, ProjectionError> {
    let zones = lir.waiting_zones().map(|view| {
        WaitingZone::new(
            ids.waiting_zones[view.ordinal().index()].clone(),
            ids.maneuver_paths[view.maneuver_path().index()].clone(),
            ids.maneuver_gates[view.entry_gate().index()].clone(),
            ids.maneuver_gates[view.release_gate().index()].clone(),
            view.max_occupancy(),
        )
    });
    Ok(WaitingRegistry::try_new(junctions, signals, zones)?)
}

fn project_parking(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
) -> Result<ParkingRegistry, ProjectionError> {
    let areas = lir
        .parking_areas()
        .map(|view| ParkingArea::new(ids.parking_areas[view.ordinal().index()].clone()));
    let spaces = lir.parking_spaces().map(|view| {
        let entry = view.entry();
        let exit = view.exit();
        let geometry = view.geometry();
        ParkingSpace::new(
            ids.parking_spaces[view.ordinal().index()].clone(),
            view.parking_area()
                .map(|area| ids.parking_areas[area.index()].clone()),
            ids.lane_edges[entry.lane_edge().index()].clone(),
            entry.progress_meters(),
            ids.lane_edges[exit.lane_edge().index()].clone(),
            exit.progress_meters(),
            ParkingSpaceGeometry::new(
                geometry.lateral_offset_meters(),
                geometry.heading_offset_radians(),
                geometry.length_meters(),
                geometry.width_meters(),
            ),
        )
    });
    Ok(ParkingRegistry::try_new(lane_graph, areas, spaces)?)
}

fn project_participant_classes(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
) -> Result<ParticipantClassRegistry, ProjectionError> {
    let classes = lir
        .participant_classes()
        .map(|view| {
            ParticipantClass::new(
                ids.participant_classes[view.ordinal().index()].clone(),
                view.parent()
                    .map(|parent| ids.participant_classes[parent.index()].as_str()),
            )
        })
        .collect();
    Ok(ParticipantClassRegistry::try_new(classes)?)
}

fn project_vehicle_profiles(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    classes: &ParticipantClassRegistry,
) -> Result<VehicleProfileRegistry, ProjectionError> {
    let profiles = lir
        .vehicle_profiles()
        .map(|view| -> Result<_, ProjectionError> {
            let class_id = &ids.participant_classes[view.participant_class().index()];
            let class = classes
                .class_handle(class_id)
                .expect("projected ParticipantClass ID must resolve");
            Ok(VehicleProfile::try_new_iidm(
                ids.vehicle_profiles[view.ordinal().index()].clone(),
                class,
                IidmProfileSpec {
                    length: view.length_meters(),
                    desired_speed: view.desired_speed_meters_per_second(),
                    min_gap: view.min_gap_meters(),
                    time_headway: view.time_headway_seconds(),
                    max_acceleration: view.max_acceleration_meters_per_second_squared(),
                    comfortable_deceleration: view
                        .comfortable_deceleration_meters_per_second_squared(),
                    emergency_deceleration: view.emergency_deceleration_meters_per_second_squared(),
                },
            )?)
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    Ok(VehicleProfileRegistry::try_new(classes, profiles)?)
}

fn project_access(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
    junctions: &JunctionRegistry,
    cross_section: &CrossSectionRegistry,
    classes: &ParticipantClassRegistry,
) -> Result<AccessRegistry, ProjectionError> {
    let rules = lir
        .access_rules()
        .map(|view| {
            let target = match view.target() {
                CanonicalAccessTarget::LaneEdge(target) => {
                    AccessTargetId::lane_edge(ids.lane_edges[target.index()].clone())
                }
                CanonicalAccessTarget::LaneGroup(target) => {
                    AccessTargetId::lane_group(ids.lane_groups[target.index()].clone())
                }
                CanonicalAccessTarget::RoadSection(target) => {
                    AccessTargetId::road_section(ids.road_sections[target.index()].clone())
                }
                CanonicalAccessTarget::ManeuverPath(target) => {
                    AccessTargetId::maneuver_path(ids.maneuver_paths[target.index()].clone())
                }
                _ => unreachable!(
                    "compiler and integration projection use the same closed LIR version"
                ),
            };
            let effect = match view.effect() {
                laneflow_compiler::AccessEffect::Allow => AccessEffect::Allow,
                laneflow_compiler::AccessEffect::Deny => AccessEffect::Deny,
                _ => unreachable!(
                    "compiler and integration projection use the same closed LIR version"
                ),
            };
            let mut rule = AccessRule::new(
                ids.access_rules[view.ordinal().index()].clone(),
                target,
                effect,
                view.participant_classes()
                    .iter()
                    .map(|class| ids.participant_classes[class.index()].clone()),
            )
            .with_priority(i64::from(view.priority()));
            if let Some(regulation) = view.regulation() {
                rule = rule.with_regulation(
                    regulation.jurisdiction(),
                    regulation.version(),
                    regulation.source(),
                );
            }
            rule
        })
        .collect();
    Ok(AccessRegistry::try_new(
        lane_graph,
        junctions,
        cross_section,
        classes,
        rules,
    )?)
}

fn project_routes(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
) -> Result<Vec<Route>, ProjectionError> {
    lir.static_routes()
        .map(|view| {
            Ok(Route::try_new(
                ids.static_routes[view.ordinal().index()].clone(),
                view.edges()
                    .iter()
                    .map(|edge| ids.lane_edges[edge.index()].clone()),
            )?)
        })
        .collect()
}

fn project_spatial(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
) -> Result<Option<SpatialRegistry>, ProjectionError> {
    let geometry_count = lir
        .lane_edges()
        .filter(|edge| edge.spatial_geometry().is_some())
        .count();
    let edge_count = ids.lane_edges.len();
    if geometry_count == 0 {
        return Ok(None);
    }
    if geometry_count != edge_count {
        return Err(ProjectionError::PartialSpatialCoverage {
            lane_edge_count: edge_count,
            geometry_count,
        });
    }

    let mut frame = None;
    let mut point_tables = Vec::with_capacity(edge_count);
    for edge in lir.lane_edges() {
        let geometry = edge
            .spatial_geometry()
            .expect("complete geometry coverage was checked");
        match frame {
            Some(existing) if existing != geometry.canonical_frame() => {
                return Err(ProjectionError::MultipleCanonicalFrames);
            }
            None => frame = Some(geometry.canonical_frame()),
            Some(_) => {}
        }
        let points = geometry
            .points()
            .map(|point| CanonicalPoint3F32::try_new(point.x, point.y, point.z))
            .collect::<Result<Vec<_>, _>>()?;
        point_tables.push(points);
    }

    let frame = frame.expect("non-empty complete geometry has a frame");
    let frame_id = CanonicalFrameId::try_new(ids.canonical_frames[frame.index()].clone())?;
    let inputs = point_tables.iter().enumerate().map(|(index, points)| {
        let edge = lane_graph
            .edge_handle(&ids.lane_edges[index])
            .expect("projected LaneEdge ID must resolve");
        SpatialEdgeInput::new(edge, points)
    });
    Ok(Some(SpatialRegistry::try_new(
        lane_graph, frame_id, inputs,
    )?))
}

fn projection_mappings(lir: &ValidatedCanonicalLir, ids: &ProjectionIds) -> Vec<ProjectionMapping> {
    let mut entries = Vec::new();
    macro_rules! append {
        ($iter:expr, $current:expr) => {
            for view in $iter {
                let current_external_id: Option<&str> = $current(&view);
                entries.push(mapping(
                    view.ordinal().raw(),
                    view.stable_id(),
                    current_external_id,
                ));
            }
        };
    }

    append!(
        lir.road_corridors(),
        |view: &laneflow_compiler::CanonicalRoadCorridorView<'_>| Some(
            ids.road_corridors[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.road_sections(),
        |view: &laneflow_compiler::CanonicalRoadSectionView<'_>| Some(
            ids.road_sections[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.authoring_lanes(),
        |_view: &laneflow_compiler::CanonicalAuthoringLaneView<'_>| None
    );
    append!(
        lir.lane_edges(),
        |view: &laneflow_compiler::CanonicalLaneEdgeView<'_>| Some(
            ids.lane_edges[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.junctions(),
        |view: &laneflow_compiler::CanonicalJunctionView<'_>| Some(
            ids.junctions[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.movements(),
        |view: &laneflow_compiler::CanonicalMovementView<'_>| Some(
            ids.movements[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.maneuver_paths(),
        |view: &laneflow_compiler::CanonicalManeuverPathView<'_>| Some(
            ids.maneuver_paths[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.maneuver_gates(),
        |view: &laneflow_compiler::CanonicalManeuverGateView<'_>| Some(
            ids.maneuver_gates[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.waiting_zones(),
        |view: &laneflow_compiler::CanonicalWaitingZoneView<'_>| Some(
            ids.waiting_zones[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.stop_lines(),
        |view: &laneflow_compiler::CanonicalStopLineView<'_>| Some(
            ids.stop_lines[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.signal_groups(),
        |view: &laneflow_compiler::CanonicalSignalGroupView<'_>| Some(
            ids.signal_groups[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.signal_controllers(),
        |view: &laneflow_compiler::CanonicalSignalControllerView<'_>| Some(
            ids.signal_controllers[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.signal_phases(),
        |view: &laneflow_compiler::CanonicalSignalPhaseView<'_>| Some(
            ids.signal_phases[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.parking_areas(),
        |view: &laneflow_compiler::CanonicalParkingAreaView<'_>| Some(
            ids.parking_areas[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.parking_spaces(),
        |view: &laneflow_compiler::CanonicalParkingSpaceView<'_>| Some(
            ids.parking_spaces[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.lane_groups(),
        |view: &laneflow_compiler::CanonicalLaneGroupView<'_>| Some(
            ids.lane_groups[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.facility_bands(),
        |view: &laneflow_compiler::CanonicalFacilityBandView<'_>| Some(
            ids.facility_bands[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.participant_classes(),
        |view: &laneflow_compiler::CanonicalParticipantClassView<'_>| Some(
            ids.participant_classes[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.access_rules(),
        |view: &laneflow_compiler::CanonicalAccessRuleView<'_>| Some(
            ids.access_rules[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.vehicle_profiles(),
        |view: &laneflow_compiler::CanonicalVehicleProfileView<'_>| Some(
            ids.vehicle_profiles[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.static_routes(),
        |view: &laneflow_compiler::CanonicalStaticRouteView<'_>| Some(
            ids.static_routes[view.ordinal().index()].as_str()
        )
    );
    append!(
        lir.canonical_frames(),
        |view: &laneflow_compiler::CanonicalFrameView<'_>| Some(
            ids.canonical_frames[view.ordinal().index()].as_str()
        )
    );
    entries
}

fn mapping<K: EntityKindMarker>(
    lir_ordinal: u32,
    stable_id: StableId<K>,
    current_external_id: Option<&str>,
) -> ProjectionMapping {
    ProjectionMapping {
        entity_kind: K::KIND,
        lir_ordinal,
        stable_id: stable_id.into_untyped(),
        current_external_id: current_external_id.map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use laneflow_compiler::{
        CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits,
        Compiler, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, SourceModuleHeader,
        SourceModuleHeaderInput, StaticRouteInput, SyntheticModuleBuilder,
    };
    use laneflow_static_contract::EntityKind;

    use super::project;

    #[test]
    fn headless_lir_projects_to_current_core_without_spatial_registry() {
        let output = compile_fixture(false);
        let projection = project(output.lir()).unwrap();

        assert_eq!(projection.traffic().lane_graph().edges().len(), 2);
        assert_eq!(projection.traffic().routes().len(), 1);
        assert!(projection.spatial().is_none());
        assert_eq!(
            projection
                .mappings()
                .entries_for(EntityKind::LaneEdge)
                .count(),
            2
        );
    }

    #[test]
    fn canonical_geometry_projects_to_complete_current_spatial_registry() {
        let output = compile_fixture(true);
        let projection = project(output.lir()).unwrap();
        let spatial = projection.spatial().unwrap();

        assert_eq!(spatial.len(), 2);
        assert_eq!(
            projection
                .mappings()
                .entries_for(EntityKind::CanonicalFrame)
                .count(),
            1
        );
    }

    fn compile_fixture(with_geometry: bool) -> laneflow_compiler::CompilationOutput {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "test/projection",
                source_document_key: "projection.test",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(7),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 12.0,
                successors: &[LaneEdgeReference::local("edge-b")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 12.0,
                speed_limit_meters_per_second: 12.0,
                successors: &[],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-main",
                edge_sequence: &[
                    LaneEdgeReference::local("edge-a"),
                    LaneEdgeReference::local("edge-b"),
                ],
            })
            .unwrap();
        if with_geometry {
            module
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "frame-main",
                    lane_edge_geometries: &[
                        LaneEdgeGeometryInput {
                            lane_edge: LaneEdgeReference::local("edge-a"),
                            centerline_points: &[
                                CanonicalPoint3F32Input {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                                CanonicalPoint3F32Input {
                                    x: 10.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                            ],
                        },
                        LaneEdgeGeometryInput {
                            lane_edge: LaneEdgeReference::local("edge-b"),
                            centerline_points: &[
                                CanonicalPoint3F32Input {
                                    x: 10.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                                CanonicalPoint3F32Input {
                                    x: 22.0,
                                    y: 0.0,
                                    z: 0.0,
                                },
                            ],
                        },
                    ],
                })
                .unwrap();
        }

        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().unwrap()).unwrap();
        Compiler::new().compile(unit.build().unwrap()).unwrap()
    }
}
