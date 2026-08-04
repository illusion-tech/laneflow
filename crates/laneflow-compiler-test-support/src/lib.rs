#![doc = include_str!("../README.md")]

use std::{error::Error, fmt};

use laneflow_compiler::{
    CanonicalAccessTarget, CanonicalCorridorElement, CanonicalSignalControl, ValidatedCanonicalLir,
};
use laneflow_core::{
    AccessEffect, AccessRegistry, AccessRule, AccessTargetId, CoreWorld, CorridorElementId,
    CrossSectionRegistry, EdgeLength, EdgeProgress, FacilityBand, IidmProfileSpec,
    InitialTrafficData, Junction, JunctionRegistry, LaneEdge, LaneGraph, LaneGroup, ManeuverGate,
    ManeuverPath, Movement, ParkingArea, ParkingRegistry, ParkingSpace, ParkingSpaceGeometry,
    ParticipantClass, ParticipantClassRegistry, RoadCorridor, RoadSection, Route, SectionLane,
    SignalAspect, SignalControlInput, SignalController, SignalGroup, SignalGroupState, SignalPhase,
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
    /// LIR 预计算表与 current Core/Spatial 从权威源字段独立重建的结果不一致。
    PrecomputedDataMismatch {
        /// 发生不一致的静态路线或实体 external ID。
        entity_id: Box<str>,
        /// 不一致的 LIR 表或字段名。
        table: &'static str,
        /// 表内记录下标；表长或实体级字段不一致时为 `None`。
        record_index: Option<usize>,
    },
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
            Self::PrecomputedDataMismatch {
                entity_id,
                table,
                record_index,
            } => {
                write!(
                    formatter,
                    "Canonical LIR 预计算数据与独立重建结果不一致：实体 {entity_id}，表 {table}"
                )?;
                if let Some(index) = record_index {
                    write!(formatter, "，记录 {index}")?;
                }
                Ok(())
            }
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
            Self::PartialSpatialCoverage { .. }
            | Self::MultipleCanonicalFrames
            | Self::PrecomputedDataMismatch { .. } => None,
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
/// 当前 Core/Spatial 构造器拒绝投影，空间几何覆盖不完整，合法 LIR 使用多个
/// canonical frame 而当前 `SpatialRegistry` 无法表达，或 LIR 预计算表与独立重建结果
/// 不一致时返回错误。
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
    validate_route_precomputes(lir, &ids, &traffic)?;
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

type RouteOccurrenceRef = (u32, u32);

fn validate_route_precomputes(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    traffic: &InitialTrafficData,
) -> Result<(), ProjectionError> {
    // `CoreWorld` 只消费由边序列构造的 current Route，并独立编译全部 occurrence 表；
    // 不把 LIR 预计算结果喂给预言机，避免错误的 LIR 表自证正确。
    let world = CoreWorld::with_traffic_data(1, traffic.clone(), Vec::new())?;
    let mut edge_reverse = vec![Vec::new(); ids.lane_edges.len()];
    let mut path_reverse = vec![Vec::new(); ids.maneuver_paths.len()];
    let mut gate_reverse = vec![Vec::new(); ids.maneuver_gates.len()];
    let mut waiting_reverse = vec![Vec::new(); ids.waiting_zones.len()];

    for route in lir.static_routes() {
        let route_id = &ids.static_routes[route.ordinal().index()];
        let route_handle = world
            .route_handle(route_id)
            .ok_or_else(|| precomputed_mismatch(route_id, "staticRoutes", None))?;
        let expected_edges = route
            .edges()
            .iter()
            .map(|edge| {
                world
                    .lane_graph()
                    .edge_handle(&ids.lane_edges[edge.index()])
                    .expect("projected LaneEdge ID must resolve")
            })
            .collect::<Vec<_>>();
        if world.route_edges(route_handle) != Some(expected_edges.as_slice()) {
            return Err(precomputed_mismatch(route_id, "staticRouteEdges", None));
        }
        for (occurrence_index, edge) in route.edges().iter().copied().enumerate() {
            edge_reverse[edge.index()].push((
                route.ordinal().raw(),
                u32::try_from(occurrence_index)
                    .expect("validated route occurrence index must fit u32"),
            ));
        }

        let expected_transitions = route
            .transition_gates()
            .map(|gate| {
                gate.map(|gate| {
                    world
                        .signals()
                        .maneuver_gate_handle(&ids.maneuver_gates[gate.index()])
                        .expect("projected ManeuverGate ID must resolve")
                })
            })
            .collect::<Vec<_>>();
        if expected_transitions.len() != expected_edges.len().saturating_sub(1) {
            return Err(precomputed_mismatch(
                route_id,
                "staticRouteTransitions",
                None,
            ));
        }
        let actual_transitions = (0..expected_transitions.len())
            .map(|index| world.route_transition_gate(route_handle, index))
            .collect::<Option<Vec<_>>>();
        let Some(actual_transitions) = actual_transitions else {
            return Err(precomputed_mismatch(
                route_id,
                "staticRouteTransitions",
                None,
            ));
        };
        if let Some(index) = first_mismatch(&expected_transitions, &actual_transitions) {
            return Err(precomputed_mismatch(
                route_id,
                "staticRouteTransitions",
                Some(index),
            ));
        }

        let expected_maneuvers = route.maneuver_occurrences().collect::<Vec<_>>();
        let actual_maneuvers = world
            .route_maneuver_occurrences(route_handle)
            .expect("projected static Route must retain its occurrence tables");
        if expected_maneuvers.len() != actual_maneuvers.len() {
            return Err(precomputed_mismatch(route_id, "maneuverOccurrences", None));
        }
        for (index, (expected, actual)) in expected_maneuvers
            .iter()
            .copied()
            .zip(actual_maneuvers.iter().copied())
            .enumerate()
        {
            let path = expected.maneuver_path();
            let expected_path = world
                .junctions()
                .maneuver_path_handle(&ids.maneuver_paths[path.index()])
                .expect("projected ManeuverPath ID must resolve");
            let expected_gate_range = expected.gate_occurrence_range();
            let expected_waiting_range = expected.waiting_zone_occurrence_range();
            if actual.maneuver_path() != expected_path
                || actual.entry_route_edge_index() != expected.entry_route_edge_index() as usize
                || actual.exit_route_edge_index() != expected.exit_route_edge_index() as usize
                || actual.gate_occurrence_range()
                    != (expected_gate_range.start as usize..expected_gate_range.end as usize)
                || actual.waiting_zone_occurrence_range()
                    != (expected_waiting_range.start as usize..expected_waiting_range.end as usize)
            {
                return Err(precomputed_mismatch(
                    route_id,
                    "maneuverOccurrences",
                    Some(index),
                ));
            }
            path_reverse[path.index()].push((
                route.ordinal().raw(),
                u32::try_from(index).expect("validated maneuver occurrence index must fit u32"),
            ));
        }

        let expected_gates = route.gate_occurrences().collect::<Vec<_>>();
        let actual_gates = world
            .route_gate_occurrences(route_handle)
            .expect("projected static Route must retain its gate occurrence table");
        if expected_gates.len() != actual_gates.len() {
            return Err(precomputed_mismatch(route_id, "gateOccurrences", None));
        }
        for (index, (expected, actual)) in expected_gates
            .iter()
            .copied()
            .zip(actual_gates.iter().copied())
            .enumerate()
        {
            let gate = expected.maneuver_gate();
            let expected_gate = world
                .signals()
                .maneuver_gate_handle(&ids.maneuver_gates[gate.index()])
                .expect("projected ManeuverGate ID must resolve");
            if actual.gate() != expected_gate
                || actual.maneuver_occurrence_index()
                    != expected.maneuver_occurrence_index() as usize
                || actual.from_route_edge_index() != expected.from_route_edge_index() as usize
                || actual.next_gate_occurrence_index()
                    != expected
                        .next_gate_occurrence_index()
                        .map(|value| value as usize)
                || actual.next_boundary_route_edge_index()
                    != expected.next_boundary_route_edge_index() as usize
                || actual.waiting_zone_occurrence_index()
                    != expected
                        .waiting_zone_occurrence_index()
                        .map(|value| value as usize)
            {
                return Err(precomputed_mismatch(
                    route_id,
                    "gateOccurrences",
                    Some(index),
                ));
            }
            gate_reverse[gate.index()].push((
                route.ordinal().raw(),
                u32::try_from(index).expect("validated gate occurrence index must fit u32"),
            ));
        }

        let expected_waiting = route.waiting_zone_occurrences().collect::<Vec<_>>();
        let actual_waiting = world
            .route_waiting_zone_occurrences(route_handle)
            .expect("projected static Route must retain its waiting-zone occurrence table");
        if expected_waiting.len() != actual_waiting.len() {
            return Err(precomputed_mismatch(
                route_id,
                "waitingZoneOccurrences",
                None,
            ));
        }
        for (index, (expected, actual)) in expected_waiting
            .iter()
            .copied()
            .zip(actual_waiting.iter().copied())
            .enumerate()
        {
            let waiting_zone = expected.waiting_zone();
            let expected_waiting_zone = world
                .waiting()
                .waiting_zone_handle(&ids.waiting_zones[waiting_zone.index()])
                .expect("projected WaitingZone ID must resolve");
            if actual.waiting_zone() != expected_waiting_zone
                || actual.maneuver_occurrence_index()
                    != expected.maneuver_occurrence_index() as usize
                || actual.entry_gate_occurrence_index()
                    != expected.entry_gate_occurrence_index() as usize
                || actual.release_gate_occurrence_index()
                    != expected.release_gate_occurrence_index() as usize
                || actual.entry_route_edge_index() != expected.entry_route_edge_index() as usize
                || actual.release_route_edge_index() != expected.release_route_edge_index() as usize
            {
                return Err(precomputed_mismatch(
                    route_id,
                    "waitingZoneOccurrences",
                    Some(index),
                ));
            }
            waiting_reverse[waiting_zone.index()].push((
                route.ordinal().raw(),
                u32::try_from(index).expect("validated waiting-zone occurrence index must fit u32"),
            ));
        }
    }

    validate_reverse_occurrences(
        lir,
        ids,
        &edge_reverse,
        &path_reverse,
        &gate_reverse,
        &waiting_reverse,
    )
}

fn validate_reverse_occurrences(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    edge_reverse: &[Vec<RouteOccurrenceRef>],
    path_reverse: &[Vec<RouteOccurrenceRef>],
    gate_reverse: &[Vec<RouteOccurrenceRef>],
    waiting_reverse: &[Vec<RouteOccurrenceRef>],
) -> Result<(), ProjectionError> {
    for edge in lir.lane_edges() {
        validate_reverse_occurrence_table(
            &ids.lane_edges[edge.ordinal().index()],
            "laneEdgeRouteOccurrences",
            edge.static_route_occurrences(),
            &edge_reverse[edge.ordinal().index()],
        )?;
    }
    for path in lir.maneuver_paths() {
        validate_reverse_occurrence_table(
            &ids.maneuver_paths[path.ordinal().index()],
            "maneuverPathRouteOccurrences",
            path.static_route_occurrences(),
            &path_reverse[path.ordinal().index()],
        )?;
    }
    for gate in lir.maneuver_gates() {
        validate_reverse_occurrence_table(
            &ids.maneuver_gates[gate.ordinal().index()],
            "maneuverGateRouteOccurrences",
            gate.static_route_occurrences(),
            &gate_reverse[gate.ordinal().index()],
        )?;
    }
    for waiting_zone in lir.waiting_zones() {
        validate_reverse_occurrence_table(
            &ids.waiting_zones[waiting_zone.ordinal().index()],
            "waitingZoneRouteOccurrences",
            waiting_zone.static_route_occurrences(),
            &waiting_reverse[waiting_zone.ordinal().index()],
        )?;
    }
    Ok(())
}

fn validate_reverse_occurrence_table(
    entity_id: &str,
    table: &'static str,
    actual: impl ExactSizeIterator<Item = laneflow_compiler::CanonicalStaticRouteOccurrenceRef>,
    expected: &[RouteOccurrenceRef],
) -> Result<(), ProjectionError> {
    let actual = actual
        .map(|occurrence| {
            (
                occurrence.static_route().raw(),
                occurrence.occurrence_index(),
            )
        })
        .collect::<Vec<_>>();
    if actual.len() != expected.len() {
        return Err(precomputed_mismatch(entity_id, table, None));
    }
    if let Some(index) = first_mismatch(expected, &actual) {
        return Err(precomputed_mismatch(entity_id, table, Some(index)));
    }
    Ok(())
}

fn first_mismatch<T: PartialEq>(expected: &[T], actual: &[T]) -> Option<usize> {
    expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
}

fn precomputed_mismatch(
    entity_id: &str,
    table: &'static str,
    record_index: Option<usize>,
) -> ProjectionError {
    ProjectionError::PrecomputedDataMismatch {
        entity_id: entity_id.into(),
        table,
        record_index,
    }
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
    let registry = SpatialRegistry::try_new(lane_graph, frame_id, inputs)?;
    validate_spatial_precomputes(lir, ids, lane_graph, &registry)?;
    Ok(Some(registry))
}

fn validate_spatial_precomputes(
    lir: &ValidatedCanonicalLir,
    ids: &ProjectionIds,
    lane_graph: &LaneGraph,
    registry: &SpatialRegistry,
) -> Result<(), ProjectionError> {
    for edge in lir.lane_edges() {
        let edge_id = &ids.lane_edges[edge.ordinal().index()];
        let geometry = edge
            .spatial_geometry()
            .expect("complete geometry coverage was checked");
        let points = geometry.points().collect::<Vec<_>>();
        let segments = geometry.segments().collect::<Vec<_>>();
        if segments.len() != points.len().saturating_sub(1) {
            return Err(precomputed_mismatch(edge_id, "spatialSegments", None));
        }

        let mut cumulative = 0.0_f32;
        for (index, (segment, pair)) in segments.iter().zip(points.windows(2)).enumerate() {
            // 与 current Spatial 从点表建表时相同，只把权威点作为输入；LIR 的 length、
            // cumulative、tangent 和 up 均不参与重算，因而不能掩盖自身损坏。
            let delta = [
                pair[1].x - pair[0].x,
                pair[1].y - pair[0].y,
                pair[1].z - pair[0].z,
            ];
            let length = delta[0].hypot(delta[1]).hypot(delta[2]);
            cumulative += length;
            if segment.length_meters.to_bits() != length.to_bits()
                || segment.cumulative_end_meters.to_bits() != cumulative.to_bits()
            {
                return Err(precomputed_mismatch(
                    edge_id,
                    "spatialSegmentLengths",
                    Some(index),
                ));
            }
        }
        if geometry.arc_length_meters().to_bits() != cumulative.to_bits() {
            return Err(precomputed_mismatch(
                edge_id,
                "spatialArcLengthMeters",
                None,
            ));
        }

        let edge_handle = lane_graph
            .edge_handle(edge_id)
            .expect("projected LaneEdge ID must resolve");
        let core_length = lane_graph
            .edge_length(edge_handle)
            .expect("projected LaneEdge handle must retain its length")
            .value();
        let mut cumulative_start = 0.0_f32;
        for (index, segment) in segments.iter().enumerate() {
            // 取段内中点而非边界，避免顶点的“出段切向量”规则把相邻段混入比较。
            let geometry_midpoint = cumulative_start + segment.length_meters * 0.5;
            let progress = EdgeProgress::try_new(
                core_length * (f64::from(geometry_midpoint) / f64::from(cumulative)),
            )?;
            let pose = registry.sample(edge_handle, progress)?;
            let tangent = pose.tangent();
            let up = pose.up();
            if [
                tangent.x().to_bits(),
                tangent.y().to_bits(),
                tangent.z().to_bits(),
            ] != segment.tangent.map(f32::to_bits)
                || [up.x().to_bits(), up.y().to_bits(), up.z().to_bits()]
                    != segment.up.map(f32::to_bits)
            {
                return Err(precomputed_mismatch(
                    edge_id,
                    "spatialSegmentBasis",
                    Some(index),
                ));
            }
            cumulative_start = segment.cumulative_end_meters;
        }
    }
    Ok(())
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
