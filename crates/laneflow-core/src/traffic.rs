//! 已通过 Core domain validation 的初始静态交通输入。

use indexmap::IndexSet;

use crate::{
    access::AccessRegistry,
    cross_section::CrossSectionRegistry,
    error::CoreError,
    graph::LaneGraph,
    handle::{EdgeHandle, ManeuverGateHandle, ManeuverPathHandle},
    junction::JunctionRegistry,
    parking::ParkingRegistry,
    participant_class::ParticipantClassRegistry,
    profile::VehicleProfileRegistry,
    route::Route,
    signal::SignalRegistry,
};

/// Route 中一次完整 ManeuverPath match 的 route-shared metadata。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManeuverOccurrence {
    maneuver_path: ManeuverPathHandle,
    entry_route_edge_index: usize,
    exit_route_edge_index: usize,
}

impl ManeuverOccurrence {
    /// 返回 occurrence 对应的 normalized ManeuverPath。
    pub const fn maneuver_path(self) -> ManeuverPathHandle {
        self.maneuver_path
    }

    /// 返回 entry edge 在 Route sequence 中的 occurrence index。
    pub const fn entry_route_edge_index(self) -> usize {
        self.entry_route_edge_index
    }

    /// 返回 exit edge 在 Route sequence 中的 occurrence index。
    pub const fn exit_route_edge_index(self) -> usize {
        self.exit_route_edge_index
    }
}

/// 已完成 graph、ManeuverPath 与 Gate 编译的 Route definition。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledRoute {
    pub(crate) definition: Route,
    pub(crate) edge_handles: Vec<EdgeHandle>,
    pub(crate) transition_gates: Vec<Option<ManeuverGateHandle>>,
    pub(crate) maneuver_occurrences: Vec<ManeuverOccurrence>,
}

impl CompiledRoute {
    pub(crate) const fn definition(&self) -> &Route {
        &self.definition
    }
}

/// 可用于初始化 Core world 的已验证静态交通输入。
#[derive(Clone, Debug, PartialEq)]
pub struct InitialTrafficData {
    lane_graph: LaneGraph,
    routes: Vec<CompiledRoute>,
    vehicle_profiles: VehicleProfileRegistry,
    junctions: JunctionRegistry,
    signals: SignalRegistry,
    parking: ParkingRegistry,
    participant_classes: ParticipantClassRegistry,
    cross_section: CrossSectionRegistry,
    access: AccessRegistry,
}

impl InitialTrafficData {
    /// 创建不含任何 static traffic definition 的初始交通输入。
    pub fn empty() -> Self {
        Self {
            lane_graph: LaneGraph::empty(),
            routes: Vec::new(),
            vehicle_profiles: VehicleProfileRegistry::empty(),
            junctions: JunctionRegistry::empty(),
            signals: SignalRegistry::empty(),
            parking: ParkingRegistry::empty(),
            participant_classes: ParticipantClassRegistry::empty(),
            cross_section: CrossSectionRegistry::empty(),
            access: AccessRegistry::empty(),
        }
    }

    /// 创建并校验全部 current static traffic data。
    ///
    /// Final assembly 总是按 retained external definitions 对最终 LaneGraph 重新绑定
    /// Junction、Signals、Parking 与 CrossSection，然后使用同一 compiler 编译 initial
    /// Routes。Access 必须最后 rebind：它消费 rebind 后的 Junction 与 CrossSection
    /// registry，顺序错误会静默混用 handle。
    #[expect(
        clippy::too_many_arguments,
        reason = "final assembly 需要全部 static domain registry"
    )]
    pub fn try_new<I>(
        lane_graph: LaneGraph,
        routes: I,
        vehicle_profiles: VehicleProfileRegistry,
        junctions: JunctionRegistry,
        signals: SignalRegistry,
        parking: ParkingRegistry,
        participant_classes: ParticipantClassRegistry,
        cross_section: CrossSectionRegistry,
        access: AccessRegistry,
    ) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = Route>,
    {
        let junctions = junctions.rebind_to_lane_graph(&lane_graph)?;
        let signals = signals.rebind_to_static_topology(&lane_graph, &junctions)?;
        let parking = parking.rebind_to_lane_graph(&lane_graph)?;
        let cross_section = cross_section.rebind_to_lane_graph(&lane_graph)?;
        let access = access.rebind(
            &lane_graph,
            &junctions,
            &cross_section,
            &participant_classes,
        )?;
        let mut route_ids = IndexSet::new();
        let mut compiled_routes = Vec::new();

        for route in routes {
            if !route_ids.insert(route.id().to_owned()) {
                return Err(CoreError::DuplicateRouteId {
                    route_id: route.id().to_owned(),
                });
            }
            compiled_routes.push(compile_route(&lane_graph, &junctions, &signals, route)?);
        }

        Ok(Self {
            lane_graph,
            routes: compiled_routes,
            vehicle_profiles,
            junctions,
            signals,
            parking,
            participant_classes,
            cross_section,
            access,
        })
    }

    /// 返回已验证的 lane graph。
    pub const fn lane_graph(&self) -> &LaneGraph {
        &self.lane_graph
    }

    /// 返回初始 route definitions，保持输入顺序。
    pub fn routes(&self) -> impl ExactSizeIterator<Item = &Route> {
        self.routes.iter().map(CompiledRoute::definition)
    }

    /// 返回 immutable Vehicle Profile registry。
    pub const fn vehicle_profiles(&self) -> &VehicleProfileRegistry {
        &self.vehicle_profiles
    }

    /// 返回 immutable Junction registry。
    pub const fn junctions(&self) -> &JunctionRegistry {
        &self.junctions
    }

    /// 返回 immutable Signals registry。
    pub const fn signals(&self) -> &SignalRegistry {
        &self.signals
    }

    /// 返回 immutable Parking registry。
    pub const fn parking(&self) -> &ParkingRegistry {
        &self.parking
    }

    /// 返回 immutable ParticipantClass registry。
    pub const fn participant_classes(&self) -> &ParticipantClassRegistry {
        &self.participant_classes
    }

    /// 返回 immutable CrossSection registry。
    pub const fn cross_section(&self) -> &CrossSectionRegistry {
        &self.cross_section
    }

    /// 返回 immutable Access registry。
    pub const fn access(&self) -> &AccessRegistry {
        &self.access
    }

    /// 拆分为 Core-owned parts。
    pub(crate) fn into_parts(
        self,
    ) -> (
        LaneGraph,
        Vec<CompiledRoute>,
        VehicleProfileRegistry,
        JunctionRegistry,
        SignalRegistry,
        ParkingRegistry,
        ParticipantClassRegistry,
        CrossSectionRegistry,
        AccessRegistry,
    ) {
        (
            self.lane_graph,
            self.routes,
            self.vehicle_profiles,
            self.junctions,
            self.signals,
            self.parking,
            self.participant_classes,
            self.cross_section,
            self.access,
        )
    }
}

pub(crate) fn compile_route(
    lane_graph: &LaneGraph,
    junctions: &JunctionRegistry,
    signals: &SignalRegistry,
    route: Route,
) -> Result<CompiledRoute, CoreError> {
    let edge_handles = resolve_route_edges(lane_graph, signals, &route)?;
    let first_edge = edge_handles[0];
    if junctions.internal_edge_owner(first_edge).is_some() {
        return Err(CoreError::RouteStartsInsideJunction {
            route_id: route.id().to_owned(),
            edge_id: lane_graph
                .edge_external_id(first_edge)
                .expect("resolved route edge must exist")
                .to_owned(),
        });
    }
    let final_edge = *edge_handles
        .last()
        .expect("Route constructor guarantees at least one edge");
    if junctions.internal_edge_owner(final_edge).is_some() {
        return Err(CoreError::RouteEndsInsideJunction {
            route_id: route.id().to_owned(),
            edge_id: lane_graph
                .edge_external_id(final_edge)
                .expect("resolved route edge must exist")
                .to_owned(),
        });
    }

    let mut transition_gates = vec![None; edge_handles.len().saturating_sub(1)];
    let mut maneuver_occurrences = Vec::new();
    let mut internal_coverage = vec![None::<ManeuverPathHandle>; edge_handles.len()];

    for entry_route_edge_index in 0..edge_handles.len().saturating_sub(1) {
        let from_edge = edge_handles[entry_route_edge_index];
        let to_edge = edge_handles[entry_route_edge_index + 1];
        let candidates = junctions.entry_transition_candidates(from_edge, to_edge);
        if candidates.is_empty() {
            continue;
        }

        let mut matches = candidates.iter().copied().filter(|candidate| {
            let candidate_edges = junctions
                .maneuver_path_edges(*candidate)
                .expect("candidate ManeuverPath must exist");
            edge_handles[entry_route_edge_index..].starts_with(candidate_edges)
        });
        let Some(maneuver_path) = matches.next() else {
            return Err(CoreError::RouteManeuverNoFullMatch {
                route_id: route.id().to_owned(),
                entry_route_edge_index,
                from_edge_id: lane_graph
                    .edge_external_id(from_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
                to_edge_id: lane_graph
                    .edge_external_id(to_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
                candidate_count: candidates.len(),
            });
        };
        if let Some(second) = matches.next() {
            return Err(CoreError::RouteManeuverMultipleFullMatches {
                route_id: route.id().to_owned(),
                entry_route_edge_index,
                first_maneuver_path_id: junctions
                    .maneuver_path_external_id(maneuver_path)
                    .expect("matched ManeuverPath must exist")
                    .to_owned(),
                second_maneuver_path_id: junctions
                    .maneuver_path_external_id(second)
                    .expect("matched ManeuverPath must exist")
                    .to_owned(),
            });
        }

        let path_edges = junctions
            .maneuver_path_edges(maneuver_path)
            .expect("matched ManeuverPath must exist");
        let exit_route_edge_index = entry_route_edge_index + path_edges.len() - 1;
        for (route_edge_index, coverage) in internal_coverage
            .iter_mut()
            .enumerate()
            .take(exit_route_edge_index)
            .skip(entry_route_edge_index + 1)
        {
            if let Some(first) = *coverage {
                return Err(CoreError::RouteManeuverInternalOverlap {
                    route_id: route.id().to_owned(),
                    route_edge_index,
                    first_maneuver_path_id: junctions
                        .maneuver_path_external_id(first)
                        .expect("covered ManeuverPath must exist")
                        .to_owned(),
                    second_maneuver_path_id: junctions
                        .maneuver_path_external_id(maneuver_path)
                        .expect("matched ManeuverPath must exist")
                        .to_owned(),
                });
            }
            *coverage = Some(maneuver_path);
        }

        transition_gates[entry_route_edge_index] =
            signals.maneuver_gate_for_path_transition(maneuver_path, 0);
        maneuver_occurrences.push(ManeuverOccurrence {
            maneuver_path,
            entry_route_edge_index,
            exit_route_edge_index,
        });
    }

    for (route_edge_index, edge) in edge_handles.iter().copied().enumerate() {
        if junctions.internal_edge_owner(edge).is_some()
            && internal_coverage[route_edge_index].is_none()
        {
            return Err(CoreError::RouteInternalEdgeUncovered {
                route_id: route.id().to_owned(),
                route_edge_index,
                edge_id: lane_graph
                    .edge_external_id(edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
            });
        }
    }

    Ok(CompiledRoute {
        definition: route,
        edge_handles,
        transition_gates,
        maneuver_occurrences,
    })
}

fn resolve_route_edges(
    lane_graph: &LaneGraph,
    signals: &SignalRegistry,
    route: &Route,
) -> Result<Vec<EdgeHandle>, CoreError> {
    let mut edge_handles = Vec::with_capacity(route.edge_ids().len());
    for edge_id in route.edge_ids() {
        let edge = lane_graph
            .edge_handle(edge_id)
            .ok_or_else(|| CoreError::UnknownRouteEdge {
                route_id: route.id().to_owned(),
                edge_id: edge_id.clone(),
            })?;
        edge_handles.push(edge);
    }

    for [from_edge, to_edge] in edge_handles.array_windows::<2>() {
        if !lane_graph.can_traverse(*from_edge, *to_edge) {
            return Err(CoreError::DisconnectedRouteEdge {
                route_id: route.id().to_owned(),
                from_edge_id: lane_graph
                    .edge_external_id(*from_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
                to_edge_id: lane_graph
                    .edge_external_id(*to_edge)
                    .expect("resolved route edge must exist")
                    .to_owned(),
            });
        }
    }

    let final_edge = *edge_handles
        .last()
        .expect("Route constructor guarantees at least one edge");
    if let Some(stop_line) = signals.stop_line_for_edge(final_edge) {
        return Err(CoreError::RouteTerminatesAtStopLine {
            route_id: route.id().to_owned(),
            edge_id: lane_graph
                .edge_external_id(final_edge)
                .expect("resolved route edge must exist")
                .to_owned(),
            stop_line_id: signals
                .stop_line_external_id(stop_line)
                .expect("resolved StopLine handle must exist")
                .to_owned(),
        });
    }

    Ok(edge_handles)
}
