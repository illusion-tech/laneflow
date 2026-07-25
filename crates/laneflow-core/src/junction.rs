//! Junction、Movement 与 ManeuverPath static topology normalization。

use std::ops::Range;

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    graph::LaneGraph,
    handle::{EdgeHandle, JunctionHandle, ManeuverPathHandle, MovementHandle},
    id::validate_external_id,
};

const MAX_STATIC_ENTITY_COUNT: usize = u32::MAX as usize;

/// Junction 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Junction {
    id: String,
}

impl Junction {
    /// 创建 Junction。ID 语法和唯一性由 `JunctionRegistry::try_new` 校验。
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// 返回 Junction external ID。
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// 道路级 Movement 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Movement {
    id: String,
    junction_id: String,
}

impl Movement {
    /// 创建 Movement。parent 引用由 `JunctionRegistry::try_new` 校验。
    pub fn new(id: impl Into<String>, junction_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            junction_id: junction_id.into(),
        }
    }

    /// 返回 Movement external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 parent Junction external ID。
    pub fn junction_id(&self) -> &str {
        &self.junction_id
    }
}

/// lane-level ManeuverPath 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManeuverPath {
    id: String,
    movement_id: String,
    entry_edge_id: String,
    internal_edge_ids: Vec<String>,
    exit_edge_id: String,
}

impl ManeuverPath {
    /// 创建 ManeuverPath。edge 引用和连通性由 `JunctionRegistry::try_new` 校验。
    pub fn new<I, S>(
        id: impl Into<String>,
        movement_id: impl Into<String>,
        entry_edge_id: impl Into<String>,
        internal_edge_ids: I,
        exit_edge_id: impl Into<String>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            movement_id: movement_id.into(),
            entry_edge_id: entry_edge_id.into(),
            internal_edge_ids: internal_edge_ids.into_iter().map(Into::into).collect(),
            exit_edge_id: exit_edge_id.into(),
        }
    }

    /// 返回 ManeuverPath external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 parent Movement external ID。
    pub fn movement_id(&self) -> &str {
        &self.movement_id
    }

    /// 返回 entry edge external ID。
    pub fn entry_edge_id(&self) -> &str {
        &self.entry_edge_id
    }

    /// 返回 ordered internal edge external IDs。
    pub fn internal_edge_ids(&self) -> &[String] {
        &self.internal_edge_ids
    }

    /// 返回 exit edge external ID。
    pub fn exit_edge_id(&self) -> &str {
        &self.exit_edge_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedJunction {
    definition: Junction,
    movements: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedMovement {
    definition: Movement,
    junction: JunctionHandle,
    maneuver_paths: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedManeuverPath {
    definition: ManeuverPath,
    movement: MovementHandle,
    edges: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryCandidateRange {
    candidates: Range<usize>,
}

#[derive(Clone, Debug)]
struct PathScratch {
    definition: ManeuverPath,
    movement: MovementHandle,
    junction: JunctionHandle,
    edges: Vec<EdgeHandle>,
}

#[derive(Clone, Debug)]
struct EdgeRoleClaim {
    path_id: String,
    junction: JunctionHandle,
}

/// `Junction -> Movement -> ManeuverPath` immutable normalized aggregate。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JunctionRegistry {
    junctions: Vec<ResolvedJunction>,
    movements: Vec<ResolvedMovement>,
    maneuver_paths: Vec<ResolvedManeuverPath>,
    junction_handles: IndexMap<String, JunctionHandle>,
    movement_handles: IndexMap<String, MovementHandle>,
    maneuver_path_handles: IndexMap<String, ManeuverPathHandle>,
    junction_movements: Vec<MovementHandle>,
    movement_maneuver_paths: Vec<ManeuverPathHandle>,
    maneuver_path_edges: Vec<EdgeHandle>,
    internal_edge_owners: Vec<Option<JunctionHandle>>,
    entry_candidate_ranges: IndexMap<(EdgeHandle, EdgeHandle), EntryCandidateRange>,
    entry_candidates: Vec<ManeuverPathHandle>,
}

impl JunctionRegistry {
    /// 创建空 static topology registry。
    pub fn empty() -> Self {
        Self {
            junctions: Vec::new(),
            movements: Vec::new(),
            maneuver_paths: Vec::new(),
            junction_handles: IndexMap::new(),
            movement_handles: IndexMap::new(),
            maneuver_path_handles: IndexMap::new(),
            junction_movements: Vec::new(),
            movement_maneuver_paths: Vec::new(),
            maneuver_path_edges: Vec::new(),
            internal_edge_owners: Vec::new(),
            entry_candidate_ranges: IndexMap::new(),
            entry_candidates: Vec::new(),
        }
    }

    /// 创建并校验 Junction、Movement 与 ManeuverPath static topology。
    pub fn try_new<J, M, P>(
        lane_graph: &LaneGraph,
        junctions: J,
        movements: M,
        maneuver_paths: P,
    ) -> Result<Self, CoreError>
    where
        J: IntoIterator<Item = Junction>,
        M: IntoIterator<Item = Movement>,
        P: IntoIterator<Item = ManeuverPath>,
    {
        let junction_definitions: Vec<Junction> = junctions.into_iter().collect();
        let movement_definitions: Vec<Movement> = movements.into_iter().collect();
        let maneuver_path_definitions: Vec<ManeuverPath> = maneuver_paths.into_iter().collect();

        validate_capacity("junctions", junction_definitions.len())?;
        validate_capacity("movements", movement_definitions.len())?;
        validate_capacity("maneuverPaths", maneuver_path_definitions.len())?;

        let mut junction_handles = IndexMap::new();
        for (index, junction) in junction_definitions.iter().enumerate() {
            validate_external_id("junctions[].id", junction.id())?;
            if junction_handles.contains_key(junction.id()) {
                return Err(CoreError::DuplicateJunctionId {
                    junction_id: junction.id().to_owned(),
                });
            }
            junction_handles.insert(junction.id().to_owned(), JunctionHandle::new(index));
        }

        let mut movement_handles = IndexMap::new();
        let mut movement_parents = Vec::with_capacity(movement_definitions.len());
        let mut junction_member_counts = vec![0_usize; junction_definitions.len()];
        for (index, movement) in movement_definitions.iter().enumerate() {
            validate_external_id("movements[].id", movement.id())?;
            if movement_handles.contains_key(movement.id()) {
                return Err(CoreError::DuplicateMovementId {
                    movement_id: movement.id().to_owned(),
                });
            }
            validate_external_id("movements[].junctionId", movement.junction_id())?;
            let junction = junction_handles
                .get(movement.junction_id())
                .copied()
                .ok_or_else(|| CoreError::UnknownMovementJunction {
                    movement_id: movement.id().to_owned(),
                    junction_id: movement.junction_id().to_owned(),
                })?;
            movement_handles.insert(movement.id().to_owned(), MovementHandle::new(index));
            movement_parents.push(junction);
            junction_member_counts[junction.index()] += 1;
        }

        let mut maneuver_path_handles = IndexMap::new();
        let mut movement_member_counts = vec![0_usize; movement_definitions.len()];
        let mut path_scratch = Vec::with_capacity(maneuver_path_definitions.len());
        let mut path_edge_count = 0_usize;
        for (index, path) in maneuver_path_definitions.into_iter().enumerate() {
            validate_external_id("maneuverPaths[].id", path.id())?;
            if maneuver_path_handles.contains_key(path.id()) {
                return Err(CoreError::DuplicateManeuverPathId {
                    maneuver_path_id: path.id().to_owned(),
                });
            }
            validate_external_id("maneuverPaths[].movementId", path.movement_id())?;
            let movement = movement_handles
                .get(path.movement_id())
                .copied()
                .ok_or_else(|| CoreError::UnknownManeuverPathMovement {
                    maneuver_path_id: path.id().to_owned(),
                    movement_id: path.movement_id().to_owned(),
                })?;
            let junction = movement_parents[movement.index()];

            let edge_count = path.internal_edge_ids().len().checked_add(2).ok_or(
                CoreError::StaticDomainCapacityExceeded {
                    domain: "maneuverPathEdgeRefs",
                    count: usize::MAX,
                    max_inclusive: u32::MAX,
                },
            )?;
            path_edge_count = path_edge_count.checked_add(edge_count).ok_or(
                CoreError::StaticDomainCapacityExceeded {
                    domain: "maneuverPathEdgeRefs",
                    count: usize::MAX,
                    max_inclusive: u32::MAX,
                },
            )?;
            validate_capacity("maneuverPathEdgeRefs", path_edge_count)?;

            let mut edges = Vec::with_capacity(edge_count);
            edges.push(resolve_path_edge(
                lane_graph,
                &path,
                "entry",
                path.entry_edge_id(),
            )?);
            for internal_edge_id in path.internal_edge_ids() {
                edges.push(resolve_path_edge(
                    lane_graph,
                    &path,
                    "internal",
                    internal_edge_id,
                )?);
            }
            edges.push(resolve_path_edge(
                lane_graph,
                &path,
                "exit",
                path.exit_edge_id(),
            )?);

            for (transition_index, pair) in edges.windows(2).enumerate() {
                if !lane_graph.can_traverse(pair[0], pair[1]) {
                    return Err(CoreError::DisconnectedManeuverPath {
                        maneuver_path_id: path.id().to_owned(),
                        transition_index,
                        from_edge_id: lane_graph
                            .edge_external_id(pair[0])
                            .expect("resolved path edge must belong to lane graph")
                            .to_owned(),
                        to_edge_id: lane_graph
                            .edge_external_id(pair[1])
                            .expect("resolved path edge must belong to lane graph")
                            .to_owned(),
                    });
                }
            }

            maneuver_path_handles.insert(path.id().to_owned(), ManeuverPathHandle::new(index));
            movement_member_counts[movement.index()] += 1;
            path_scratch.push(PathScratch {
                definition: path,
                movement,
                junction,
                edges,
            });
        }

        let mut traversal_signatures = IndexMap::<Vec<EdgeHandle>, usize>::new();
        for (index, path) in path_scratch.iter().enumerate() {
            if let Some(first_index) = traversal_signatures.get(&path.edges).copied() {
                let first = &path_scratch[first_index];
                return Err(CoreError::DuplicateManeuverPathSequence {
                    first_maneuver_path_id: first.definition.id().to_owned(),
                    first_junction_id: junction_definitions[first.junction.index()].id().to_owned(),
                    duplicate_maneuver_path_id: path.definition.id().to_owned(),
                    duplicate_junction_id: junction_definitions[path.junction.index()]
                        .id()
                        .to_owned(),
                });
            }
            traversal_signatures.insert(path.edges.clone(), index);
        }

        let edge_count = lane_graph.edges().len();
        let mut internal_claims = vec![None::<EdgeRoleClaim>; edge_count];
        let mut boundary_claims = vec![None::<EdgeRoleClaim>; edge_count];
        for path in &path_scratch {
            for boundary in [
                path.edges[0],
                *path.edges.last().expect("path has exit edge"),
            ] {
                if let Some(internal) = &internal_claims[boundary.index()] {
                    return Err(CoreError::ManeuverPathEdgeRoleConflict {
                        edge_id: lane_graph
                            .edge_external_id(boundary)
                            .expect("resolved path edge must belong to lane graph")
                            .to_owned(),
                        internal_maneuver_path_id: internal.path_id.clone(),
                        boundary_maneuver_path_id: path.definition.id().to_owned(),
                    });
                }
                boundary_claims[boundary.index()].get_or_insert_with(|| EdgeRoleClaim {
                    path_id: path.definition.id().to_owned(),
                    junction: path.junction,
                });
            }

            for internal in &path.edges[1..path.edges.len() - 1] {
                if let Some(boundary) = &boundary_claims[internal.index()] {
                    return Err(CoreError::ManeuverPathEdgeRoleConflict {
                        edge_id: lane_graph
                            .edge_external_id(*internal)
                            .expect("resolved path edge must belong to lane graph")
                            .to_owned(),
                        internal_maneuver_path_id: path.definition.id().to_owned(),
                        boundary_maneuver_path_id: boundary.path_id.clone(),
                    });
                }
                if let Some(first) = &internal_claims[internal.index()] {
                    if first.junction != path.junction {
                        return Err(CoreError::ManeuverInternalEdgeJunctionConflict {
                            edge_id: lane_graph
                                .edge_external_id(*internal)
                                .expect("resolved path edge must belong to lane graph")
                                .to_owned(),
                            first_junction_id: junction_definitions[first.junction.index()]
                                .id()
                                .to_owned(),
                            duplicate_junction_id: junction_definitions[path.junction.index()]
                                .id()
                                .to_owned(),
                        });
                    }
                } else {
                    internal_claims[internal.index()] = Some(EdgeRoleClaim {
                        path_id: path.definition.id().to_owned(),
                        junction: path.junction,
                    });
                }
            }
        }

        for (index, junction) in junction_definitions.iter().enumerate() {
            if junction_member_counts[index] == 0 {
                return Err(CoreError::EmptyJunction {
                    junction_id: junction.id().to_owned(),
                });
            }
        }
        for (index, movement) in movement_definitions.iter().enumerate() {
            if movement_member_counts[index] == 0 {
                return Err(CoreError::EmptyMovement {
                    movement_id: movement.id().to_owned(),
                });
            }
        }

        let (junction_ranges, junction_movements) = build_member_ranges(
            &junction_member_counts,
            movement_parents.iter().copied(),
            MovementHandle::new,
        );
        let path_parents = path_scratch.iter().map(|path| path.movement);
        let (movement_ranges, movement_maneuver_paths) = build_member_ranges(
            &movement_member_counts,
            path_parents,
            ManeuverPathHandle::new,
        );

        let junctions = junction_definitions
            .into_iter()
            .zip(junction_ranges)
            .map(|(definition, movements)| ResolvedJunction {
                definition,
                movements,
            })
            .collect();
        let movements = movement_definitions
            .into_iter()
            .zip(movement_parents)
            .zip(movement_ranges)
            .map(
                |((definition, junction), maneuver_paths)| ResolvedMovement {
                    definition,
                    junction,
                    maneuver_paths,
                },
            )
            .collect();

        let mut maneuver_path_edges = Vec::with_capacity(path_edge_count);
        let mut maneuver_paths = Vec::with_capacity(path_scratch.len());
        let mut entry_candidate_scratch =
            IndexMap::<(EdgeHandle, EdgeHandle), Vec<ManeuverPathHandle>>::new();
        for (index, path) in path_scratch.into_iter().enumerate() {
            let start = maneuver_path_edges.len();
            maneuver_path_edges.extend_from_slice(&path.edges);
            let end = maneuver_path_edges.len();
            let handle = ManeuverPathHandle::new(index);
            entry_candidate_scratch
                .entry((path.edges[0], path.edges[1]))
                .or_default()
                .push(handle);
            maneuver_paths.push(ResolvedManeuverPath {
                definition: path.definition,
                movement: path.movement,
                edges: start..end,
            });
        }

        let mut entry_candidate_ranges = IndexMap::new();
        let mut entry_candidates = Vec::with_capacity(maneuver_paths.len());
        for (transition, candidates) in entry_candidate_scratch {
            let start = entry_candidates.len();
            entry_candidates.extend(candidates);
            let end = entry_candidates.len();
            entry_candidate_ranges.insert(
                transition,
                EntryCandidateRange {
                    candidates: start..end,
                },
            );
        }

        Ok(Self {
            junctions,
            movements,
            maneuver_paths,
            junction_handles,
            movement_handles,
            maneuver_path_handles,
            junction_movements,
            movement_maneuver_paths,
            maneuver_path_edges,
            internal_edge_owners: internal_claims
                .into_iter()
                .map(|claim| claim.map(|claim| claim.junction))
                .collect(),
            entry_candidate_ranges,
            entry_candidates,
        })
    }

    /// 按 retained external definitions 对目标 LaneGraph 重新 normalization。
    pub fn rebind_to_lane_graph(&self, lane_graph: &LaneGraph) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            self.junctions
                .iter()
                .map(|junction| junction.definition.clone()),
            self.movements
                .iter()
                .map(|movement| movement.definition.clone()),
            self.maneuver_paths
                .iter()
                .map(|path| path.definition.clone()),
        )
    }

    /// 返回 registry 是否为空。
    pub fn is_empty(&self) -> bool {
        self.junctions.is_empty() && self.movements.is_empty() && self.maneuver_paths.is_empty()
    }

    /// 返回 Junction external ID 对应的 handle。
    pub fn junction_handle(&self, external_id: &str) -> Option<JunctionHandle> {
        self.junction_handles.get(external_id).copied()
    }

    /// 返回 Junction handle 对应的 external ID。
    pub fn junction_external_id(&self, handle: JunctionHandle) -> Option<&str> {
        self.junction(handle).map(Junction::id)
    }

    /// 返回 Movement external ID 对应的 handle。
    pub fn movement_handle(&self, external_id: &str) -> Option<MovementHandle> {
        self.movement_handles.get(external_id).copied()
    }

    /// 返回 Movement handle 对应的 external ID。
    pub fn movement_external_id(&self, handle: MovementHandle) -> Option<&str> {
        self.movement(handle).map(Movement::id)
    }

    /// 返回 ManeuverPath external ID 对应的 handle。
    pub fn maneuver_path_handle(&self, external_id: &str) -> Option<ManeuverPathHandle> {
        self.maneuver_path_handles.get(external_id).copied()
    }

    /// 返回 ManeuverPath handle 对应的 external ID。
    pub fn maneuver_path_external_id(&self, handle: ManeuverPathHandle) -> Option<&str> {
        self.maneuver_path(handle).map(ManeuverPath::id)
    }

    /// 返回指定 Junction definition。
    pub fn junction(&self, handle: JunctionHandle) -> Option<&Junction> {
        self.junctions
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 返回指定 Movement definition。
    pub fn movement(&self, handle: MovementHandle) -> Option<&Movement> {
        self.movements
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 返回指定 ManeuverPath definition。
    pub fn maneuver_path(&self, handle: ManeuverPathHandle) -> Option<&ManeuverPath> {
        self.maneuver_paths
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 normalization order 遍历 Junction handles。
    pub fn junctions(&self) -> impl ExactSizeIterator<Item = JunctionHandle> + '_ {
        (0..self.junctions.len()).map(JunctionHandle::new)
    }

    /// 按 normalization order 遍历 Movement handles。
    pub fn movements(&self) -> impl ExactSizeIterator<Item = MovementHandle> + '_ {
        (0..self.movements.len()).map(MovementHandle::new)
    }

    /// 按 normalization order 遍历 ManeuverPath handles。
    pub fn maneuver_paths(&self) -> impl ExactSizeIterator<Item = ManeuverPathHandle> + '_ {
        (0..self.maneuver_paths.len()).map(ManeuverPathHandle::new)
    }

    /// 返回 Movement 的 parent Junction。
    pub fn movement_junction(&self, handle: MovementHandle) -> Option<JunctionHandle> {
        self.movements
            .get(handle.index())
            .map(|movement| movement.junction)
    }

    /// 返回 ManeuverPath 的 parent Movement。
    pub fn maneuver_path_movement(&self, handle: ManeuverPathHandle) -> Option<MovementHandle> {
        self.maneuver_paths
            .get(handle.index())
            .map(|path| path.movement)
    }

    /// 返回 Junction 的 Movement handles，保持 Movement input order。
    pub fn junction_movements(
        &self,
        handle: JunctionHandle,
    ) -> Option<impl ExactSizeIterator<Item = MovementHandle> + '_> {
        let range = self.junctions.get(handle.index())?.movements.clone();
        Some(self.junction_movements[range].iter().copied())
    }

    /// 返回 Movement 的 ManeuverPath handles，保持 ManeuverPath input order。
    pub fn movement_maneuver_paths(
        &self,
        handle: MovementHandle,
    ) -> Option<impl ExactSizeIterator<Item = ManeuverPathHandle> + '_> {
        let range = self.movements.get(handle.index())?.maneuver_paths.clone();
        Some(self.movement_maneuver_paths[range].iter().copied())
    }

    /// 返回 ManeuverPath 的完整 resolved edge sequence。
    pub fn maneuver_path_edges(&self, handle: ManeuverPathHandle) -> Option<&[EdgeHandle]> {
        let range = self.maneuver_paths.get(handle.index())?.edges.clone();
        Some(&self.maneuver_path_edges[range])
    }

    /// 返回指定 edge 的 derived internal Junction owner。
    pub fn internal_edge_owner(&self, edge: EdgeHandle) -> Option<JunctionHandle> {
        self.internal_edge_owners
            .get(edge.index())
            .copied()
            .flatten()
    }

    /// 返回 entry transition 对应的 ManeuverPath candidates。
    pub(crate) fn entry_transition_candidates(
        &self,
        from: EdgeHandle,
        to: EdgeHandle,
    ) -> &[ManeuverPathHandle] {
        let Some(range) = self.entry_candidate_ranges.get(&(from, to)) else {
            return &[];
        };
        &self.entry_candidates[range.candidates.clone()]
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        fn string_key_map_bytes<V>(map: &IndexMap<String, V>) -> usize {
            map.capacity() * std::mem::size_of::<(String, V)>()
                + map.keys().map(String::capacity).sum::<usize>()
        }

        let Self {
            junctions,
            movements,
            maneuver_paths,
            junction_handles,
            movement_handles,
            maneuver_path_handles,
            junction_movements,
            movement_maneuver_paths,
            maneuver_path_edges,
            internal_edge_owners,
            entry_candidate_ranges,
            entry_candidates,
        } = self;

        let junction_bytes = junctions.capacity() * std::mem::size_of::<ResolvedJunction>()
            + junctions
                .iter()
                .map(|junction| junction.definition.id.capacity())
                .sum::<usize>();
        let movement_bytes = movements.capacity() * std::mem::size_of::<ResolvedMovement>()
            + movements
                .iter()
                .map(|movement| {
                    movement.definition.id.capacity() + movement.definition.junction_id.capacity()
                })
                .sum::<usize>();
        let maneuver_path_bytes = maneuver_paths.capacity()
            * std::mem::size_of::<ResolvedManeuverPath>()
            + maneuver_paths
                .iter()
                .map(|path| {
                    path.definition.id.capacity()
                        + path.definition.movement_id.capacity()
                        + path.definition.entry_edge_id.capacity()
                        + path.definition.exit_edge_id.capacity()
                        + path.definition.internal_edge_ids.capacity()
                            * std::mem::size_of::<String>()
                        + path
                            .definition
                            .internal_edge_ids
                            .iter()
                            .map(String::capacity)
                            .sum::<usize>()
                })
                .sum::<usize>();
        let resolver_bytes = string_key_map_bytes(junction_handles)
            + string_key_map_bytes(movement_handles)
            + string_key_map_bytes(maneuver_path_handles);
        let flat_index_bytes = junction_movements.capacity()
            * std::mem::size_of::<MovementHandle>()
            + movement_maneuver_paths.capacity() * std::mem::size_of::<ManeuverPathHandle>()
            + maneuver_path_edges.capacity() * std::mem::size_of::<EdgeHandle>()
            + internal_edge_owners.capacity() * std::mem::size_of::<Option<JunctionHandle>>()
            + entry_candidate_ranges.capacity()
                * std::mem::size_of::<((EdgeHandle, EdgeHandle), EntryCandidateRange)>()
            + entry_candidates.capacity() * std::mem::size_of::<ManeuverPathHandle>();

        junction_bytes + movement_bytes + maneuver_path_bytes + resolver_bytes + flat_index_bytes
    }
}

fn validate_capacity(domain: &'static str, count: usize) -> Result<(), CoreError> {
    if count > MAX_STATIC_ENTITY_COUNT {
        return Err(CoreError::StaticDomainCapacityExceeded {
            domain,
            count,
            max_inclusive: u32::MAX,
        });
    }
    Ok(())
}

fn resolve_path_edge(
    lane_graph: &LaneGraph,
    path: &ManeuverPath,
    role: &'static str,
    edge_id: &str,
) -> Result<EdgeHandle, CoreError> {
    validate_external_id(
        match role {
            "entry" => "maneuverPaths[].entryEdgeId",
            "internal" => "maneuverPaths[].internalEdgeIds[]",
            "exit" => "maneuverPaths[].exitEdgeId",
            _ => unreachable!("path role is internal invariant"),
        },
        edge_id,
    )?;
    lane_graph
        .edge_handle(edge_id)
        .ok_or_else(|| CoreError::UnknownManeuverPathEdge {
            maneuver_path_id: path.id().to_owned(),
            role,
            edge_id: edge_id.to_owned(),
        })
}

fn build_member_ranges<P, C>(
    counts: &[usize],
    parents: impl IntoIterator<Item = P>,
    child_handle: impl Fn(usize) -> C,
) -> (Vec<Range<usize>>, Vec<C>)
where
    P: Copy + IntoParentIndex,
    C: Copy,
{
    let mut starts = Vec::with_capacity(counts.len());
    let mut total = 0_usize;
    for count in counts {
        starts.push(total);
        total += count;
    }
    let ranges = starts
        .iter()
        .zip(counts)
        .map(|(start, count)| *start..*start + *count)
        .collect::<Vec<_>>();
    let mut next = starts;
    let mut members = vec![None; total];
    for (child_index, parent) in parents.into_iter().enumerate() {
        let parent_index = parent.parent_index();
        let slot = next[parent_index];
        members[slot] = Some(child_handle(child_index));
        next[parent_index] += 1;
    }
    (
        ranges,
        members
            .into_iter()
            .map(|member| member.expect("member counts must match normalized parents"))
            .collect(),
    )
}

trait IntoParentIndex: Copy {
    fn parent_index(self) -> usize;
}

impl IntoParentIndex for JunctionHandle {
    fn parent_index(self) -> usize {
        self.index()
    }
}

impl IntoParentIndex for MovementHandle {
    fn parent_index(self) -> usize {
        self.index()
    }
}
