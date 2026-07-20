//! Core edge handle 到受检折线条目的 immutable registry 边界。

use std::{collections::HashMap, fmt};

use laneflow_core::{EdgeHandle, EdgeProgress, LaneGraph};

use crate::{
    CanonicalFrameId, CanonicalPoseF32, SPATIAL_JOIN_POSITION_TOLERANCE_METERS, SpatialEdgeInput,
    SpatialError,
    geometry::{BoundPolyline, point_distance},
};

const MAX_REGISTRY_ENTRIES: usize = u32::MAX as usize;

/// 已完整绑定到一个 lane graph 的 opaque immutable Spatial registry。
///
/// 条目顺序固定为 `LaneGraph::edges()` 顺序；私有散列表只用于 handle lookup，
/// 不参与可观察迭代或错误顺序。
#[derive(Clone, PartialEq)]
pub struct SpatialRegistry {
    frame_id: CanonicalFrameId,
    edge_handles: Vec<EdgeHandle>,
    entries: Vec<BoundPolyline>,
    edge_slots: HashMap<EdgeHandle, u32>,
}

// 所有浮点字段都只能通过拒绝 NaN 的受检构造路径进入 registry。
impl Eq for SpatialRegistry {}

impl fmt::Debug for SpatialRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpatialRegistry")
            .field("frame_id", &self.frame_id)
            .field("edge_count", &self.edge_handles.len())
            .finish_non_exhaustive()
    }
}

impl SpatialRegistry {
    /// 原子构造完整的折线 registry。
    ///
    /// 输入先按调用方顺序完成 unknown、duplicate 与逐折线校验，再按
    /// `LaneGraph::edges()` 顺序检查缺失绑定，最后按 graph/next-edge 顺序检查连接端点。
    /// 任一失败都不会返回部分 registry，也不会修改 Core graph。
    ///
    /// # Errors
    ///
    /// 任一 edge 未知、重复、缺失，折线或长度无效，或已声明连接不连续时返回
    /// 对应的结构化 [`SpatialError`]。
    pub fn try_new<'a, I>(
        lane_graph: &LaneGraph,
        frame_id: CanonicalFrameId,
        edge_inputs: I,
    ) -> Result<Self, SpatialError>
    where
        I: IntoIterator<Item = SpatialEdgeInput<'a>>,
    {
        let edge_inputs: Vec<_> = edge_inputs.into_iter().collect();
        validate_registry_capacity(edge_inputs.len())?;

        let mut staged_entries = HashMap::with_capacity(edge_inputs.len());
        for input in edge_inputs {
            let edge = input.edge();
            let core_length = lane_graph
                .edge_length(edge)
                .ok_or(SpatialError::UnknownEdgeHandle { edge })?;
            if staged_entries.contains_key(&edge) {
                return Err(SpatialError::DuplicateEdgeBinding { edge });
            }

            let polyline = BoundPolyline::try_new(edge, core_length, input.points())?;
            staged_entries.insert(edge, polyline);
        }

        let edge_count = lane_graph.edges().len();
        let mut edge_handles = Vec::with_capacity(edge_count);
        let mut entries = Vec::with_capacity(edge_count);
        for edge_definition in lane_graph.edges() {
            let edge = lane_graph
                .edge_handle(edge_definition.id())
                .expect("LaneGraph::edges must resolve through its own registry");
            let entry = staged_entries
                .remove(&edge)
                .ok_or(SpatialError::MissingEdgeBinding { edge })?;
            edge_handles.push(edge);
            entries.push(entry);
        }

        let mut edge_slots = HashMap::with_capacity(edge_handles.len());
        for (index, edge) in edge_handles.iter().copied().enumerate() {
            let slot =
                u32::try_from(index).map_err(|_| registry_capacity_error(edge_handles.len()))?;
            edge_slots.insert(edge, slot);
        }

        validate_edge_joins(lane_graph, &edge_handles, &entries, &edge_slots)?;

        Ok(Self {
            frame_id,
            edge_handles,
            entries,
            edge_slots,
        })
    }

    /// 返回本 registry 唯一的标准坐标框架 ID。
    pub const fn frame_id(&self) -> &CanonicalFrameId {
        &self.frame_id
    }

    /// 返回完整绑定的 edge 数量。
    pub const fn len(&self) -> usize {
        self.edge_handles.len()
    }

    /// 返回 registry 是否绑定空 lane graph。
    pub const fn is_empty(&self) -> bool {
        self.edge_handles.is_empty()
    }

    /// 返回目标 Core edge 是否存在已提交绑定。
    pub fn contains_edge(&self, edge: EdgeHandle) -> bool {
        self.edge_slots.contains_key(&edge)
    }

    /// 使用 Core 权威进度确定性采样 canonical `f32` 位姿。
    ///
    /// 内部顶点使用出段切向量，最终端点使用最后一个入段切向量。
    ///
    /// # Errors
    ///
    /// edge 未绑定或进度严格大于 Core edge length 时返回结构化错误。
    pub fn sample(
        &self,
        edge: EdgeHandle,
        progress: EdgeProgress,
    ) -> Result<CanonicalPoseF32, SpatialError> {
        let slot = self
            .edge_slot(edge)
            .ok_or(SpatialError::UnknownEdgeHandle { edge })?;
        self.entries[slot as usize].sample(edge, progress)
    }

    pub(crate) fn edge_slot(&self, edge: EdgeHandle) -> Option<u32> {
        self.edge_slots.get(&edge).copied()
    }
}

fn validate_edge_joins(
    lane_graph: &LaneGraph,
    edge_handles: &[EdgeHandle],
    entries: &[BoundPolyline],
    edge_slots: &HashMap<EdgeHandle, u32>,
) -> Result<(), SpatialError> {
    for (from_index, from_edge) in edge_handles.iter().copied().enumerate() {
        let from_point = entries[from_index].last_point();
        let next_edges = lane_graph
            .next_edges(from_edge)
            .expect("a handle from LaneGraph::edges must resolve next edges");
        for to_edge in next_edges.iter().copied() {
            let to_slot = edge_slots
                .get(&to_edge)
                .copied()
                .expect("complete registry contains every graph edge");
            let distance_meters =
                point_distance(from_point, entries[to_slot as usize].first_point());
            if distance_meters > SPATIAL_JOIN_POSITION_TOLERANCE_METERS {
                return Err(SpatialError::DisconnectedEdgeJoin {
                    from_edge,
                    to_edge,
                    distance_meters,
                    tolerance_meters: SPATIAL_JOIN_POSITION_TOLERANCE_METERS,
                });
            }
        }
    }
    Ok(())
}

fn validate_registry_capacity(actual: usize) -> Result<(), SpatialError> {
    if actual > MAX_REGISTRY_ENTRIES {
        return Err(registry_capacity_error(actual));
    }
    Ok(())
}

fn registry_capacity_error(actual: usize) -> SpatialError {
    SpatialError::RegistryCapacityExceeded {
        actual,
        max: MAX_REGISTRY_ENTRIES,
    }
}

#[cfg(test)]
mod tests {
    use std::{hint::black_box, time::Instant};

    use laneflow_core::{EdgeLength, LaneEdge};

    use super::*;
    use crate::{
        CanonicalPoint3F32, SPATIAL_LENGTH_ABS_TOLERANCE_METERS, SPATIAL_MIN_PROJECTED_UP_LENGTH,
        SPATIAL_MIN_SEGMENT_LENGTH_METERS,
    };

    fn edge_length(value: f64) -> EdgeLength {
        EdgeLength::try_new(value).expect("valid edge length")
    }

    fn graph(edges: &[(&str, f64, &[&str])]) -> LaneGraph {
        LaneGraph::try_new(edges.iter().map(|(id, length, next)| {
            LaneEdge::new(*id, edge_length(*length), next.iter().copied())
        }))
        .expect("valid lane graph")
    }

    fn frame_id() -> CanonicalFrameId {
        CanonicalFrameId::try_new("campus/main").expect("valid frame ID")
    }

    fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32 {
        CanonicalPoint3F32::try_new(x, y, z).expect("valid point")
    }

    #[test]
    fn empty_lane_graph_commits_empty_registry() {
        let registry = SpatialRegistry::try_new(
            &LaneGraph::empty(),
            frame_id(),
            std::iter::empty::<SpatialEdgeInput<'_>>(),
        )
        .expect("empty graph has complete empty bindings");

        assert_eq!(registry.frame_id().as_str(), "campus/main");
        assert!(registry.is_empty());
    }

    #[test]
    fn registry_uses_lane_graph_order_and_private_dense_lookup() {
        let lane_graph = graph(&[("A", 1.0, &[]), ("B", 1.0, &[]), ("C", 1.0, &[])]);
        let edge_a = lane_graph.edge_handle("A").expect("edge A");
        let edge_b = lane_graph.edge_handle("B").expect("edge B");
        let edge_c = lane_graph.edge_handle("C").expect("edge C");
        let a = [point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)];
        let b = [point(0.0, 0.0, 2.0), point(1.0, 0.0, 2.0)];
        let c = [point(0.0, 0.0, 4.0), point(1.0, 0.0, 4.0)];

        let registry = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [
                SpatialEdgeInput::new(edge_c, &c),
                SpatialEdgeInput::new(edge_a, &a),
                SpatialEdgeInput::new(edge_b, &b),
            ],
        )
        .expect("complete bindings");

        assert_eq!(registry.edge_handles, [edge_a, edge_b, edge_c]);
        assert_eq!(registry.edge_slot(edge_a), Some(0));
        assert_eq!(registry.edge_slot(edge_b), Some(1));
        assert_eq!(registry.edge_slot(edge_c), Some(2));
        assert!(!format!("{registry:?}").contains("edge_slots"));
    }

    #[test]
    fn validation_order_is_unknown_duplicate_geometry_then_missing() {
        let lane_graph = graph(&[("A", 1.0, &[]), ("B", 1.0, &[])]);
        let foreign_graph = graph(&[("X", 1.0, &[]), ("Y", 1.0, &[]), ("Z", 1.0, &[])]);
        let unknown = foreign_graph.edge_handle("Z").expect("foreign edge");
        let edge_a = lane_graph.edge_handle("A").expect("edge A");
        let valid = [point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)];
        let invalid = [point(0.0, 0.0, 0.0)];

        assert_eq!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [
                    SpatialEdgeInput::new(unknown, &invalid),
                    SpatialEdgeInput::new(unknown, &invalid),
                ],
            )
            .expect_err("unknown wins"),
            SpatialError::UnknownEdgeHandle { edge: unknown }
        );
        assert_eq!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [
                    SpatialEdgeInput::new(edge_a, &valid),
                    SpatialEdgeInput::new(edge_a, &invalid),
                ],
            )
            .expect_err("duplicate wins before duplicate input geometry"),
            SpatialError::DuplicateEdgeBinding { edge: edge_a }
        );
        assert!(matches!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [SpatialEdgeInput::new(edge_a, &invalid)],
            ),
            Err(SpatialError::InsufficientPolylinePoints { edge, .. }) if edge == edge_a
        ));
        assert_eq!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [SpatialEdgeInput::new(edge_a, &valid)],
            )
            .expect_err("missing follows per-input validation"),
            SpatialError::MissingEdgeBinding {
                edge: lane_graph.edge_handle("B").expect("edge B")
            }
        );
    }

    #[test]
    fn segment_and_basis_boundaries_are_enforced() {
        let lane_graph = graph(&[("A", 0.1, &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let exact_min = [point(0.0, 0.0, 0.0), point(0.1, 0.0, 0.0)];
        assert_eq!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [SpatialEdgeInput::new(edge, &exact_min)],
            )
            .expect_err("minimum is exclusive"),
            SpatialError::DegenerateSegment {
                edge,
                segment_index: 0,
                length_meters: 0.1,
                min_exclusive_meters: SPATIAL_MIN_SEGMENT_LENGTH_METERS,
            }
        );

        let above_min = f32::from_bits(0.1_f32.to_bits() + 1);
        let above_graph = graph(&[("A", f64::from(above_min), &[])]);
        let edge = above_graph.edge_handle("A").expect("edge A");
        let points = [point(0.0, 0.0, 0.0), point(above_min, 0.0, 0.0)];
        SpatialRegistry::try_new(
            &above_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("the next f32 above the exclusive minimum is accepted");

        let vertical_graph = graph(&[("A", 1.0, &[])]);
        let edge = vertical_graph.edge_handle("A").expect("edge A");
        let vertical = [point(0.0, 0.0, 0.0), point(0.0, 1.0, 0.0)];
        assert_eq!(
            SpatialRegistry::try_new(
                &vertical_graph,
                frame_id(),
                [SpatialEdgeInput::new(edge, &vertical)],
            )
            .expect_err("vertical basis is rejected"),
            SpatialError::DegenerateBasis {
                edge,
                segment_index: 0,
                projected_up_length: 0.0,
                min_inclusive: SPATIAL_MIN_PROJECTED_UP_LENGTH,
            }
        );
    }

    #[test]
    fn length_tolerance_is_inclusive_and_relative_term_can_dominate() {
        let within = graph(&[("A", 1.009_999_999_999_999_8, &[])]);
        let edge = within.edge_handle("A").expect("edge A");
        let points = [point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)];
        SpatialRegistry::try_new(&within, frame_id(), [SpatialEdgeInput::new(edge, &points)])
            .expect("a difference representable just inside the inclusive boundary is accepted");

        let beyond = graph(&[("A", 1.010_000_000_1, &[])]);
        let edge = beyond.edge_handle("A").expect("edge A");
        assert!(matches!(
            SpatialRegistry::try_new(
                &beyond,
                frame_id(),
                [SpatialEdgeInput::new(edge, &points)],
            ),
            Err(SpatialError::LengthMismatch { difference_meters, tolerance_meters, .. })
                if difference_meters > tolerance_meters
                    && tolerance_meters == SPATIAL_LENGTH_ABS_TOLERANCE_METERS
        ));

        let long = graph(&[("A", 10_000.01, &[])]);
        let edge = long.edge_handle("A").expect("edge A");
        let long_points = [point(0.0, 0.0, 0.0), point(10_000.0, 0.0, 0.0)];
        SpatialRegistry::try_new(
            &long,
            frame_id(),
            [SpatialEdgeInput::new(edge, &long_points)],
        )
        .expect("relative tolerance equality is accepted");

        let long_beyond = graph(&[("A", 10_000.010_1, &[])]);
        let edge = long_beyond.edge_handle("A").expect("edge A");
        assert!(matches!(
            SpatialRegistry::try_new(
                &long_beyond,
                frame_id(),
                [SpatialEdgeInput::new(edge, &long_points)],
            ),
            Err(SpatialError::LengthMismatch { difference_meters, tolerance_meters, .. })
                if tolerance_meters > SPATIAL_LENGTH_ABS_TOLERANCE_METERS
                    && difference_meters > tolerance_meters
        ));
    }

    #[test]
    fn joins_use_runtime_points_and_stable_graph_order() {
        let lane_graph = graph(&[("A", 1.0, &["C"]), ("B", 1.0, &["C"]), ("C", 1.0, &[])]);
        let a_edge = lane_graph.edge_handle("A").expect("A");
        let b_edge = lane_graph.edge_handle("B").expect("B");
        let c_edge = lane_graph.edge_handle("C").expect("C");
        let a = [point(0.0, 0.0, 0.0), point(1.006, 0.0, 0.0)];
        let b = [point(0.007, 0.0, 0.0), point(1.007, 0.0, 0.0)];
        let c = [point(1.0, 0.0, 0.0), point(2.0, 0.0, 0.0)];

        assert!(matches!(
            SpatialRegistry::try_new(
                &lane_graph,
                frame_id(),
                [
                    SpatialEdgeInput::new(b_edge, &b),
                    SpatialEdgeInput::new(c_edge, &c),
                    SpatialEdgeInput::new(a_edge, &a),
                ],
            ),
            Err(SpatialError::DisconnectedEdgeJoin { from_edge, to_edge, .. })
                if from_edge == a_edge && to_edge == c_edge
        ));
    }

    #[test]
    fn join_tolerance_is_inclusive() {
        let lane_graph = graph(&[("A", 1.005, &["B"]), ("B", 1.0, &[])]);
        let a_edge = lane_graph.edge_handle("A").expect("A");
        let b_edge = lane_graph.edge_handle("B").expect("B");
        let a = [point(-1.0, 0.0, 0.0), point(0.005, 0.0, 0.0)];
        let b = [point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)];

        SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [
                SpatialEdgeInput::new(a_edge, &a),
                SpatialEdgeInput::new(b_edge, &b),
            ],
        )
        .expect("a runtime f32 endpoint distance within 5 mm is accepted");
    }

    #[test]
    fn sampling_is_right_continuous_and_uses_incoming_final_segment() {
        let lane_graph = graph(&[("A", 2.0, &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let points = [
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(1.0, 0.0, 1.0),
        ];
        let registry = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("valid registry");

        let start = registry
            .sample(edge, EdgeProgress::ZERO)
            .expect("start sample");
        assert_eq!(start.position(), points[0]);
        assert_eq!([start.tangent().x(), start.tangent().z()], [1.0, 0.0]);

        let vertex = registry
            .sample(edge, EdgeProgress::try_new(1.0).expect("valid progress"))
            .expect("vertex sample");
        assert_eq!(vertex.position(), points[1]);
        assert_eq!([vertex.tangent().x(), vertex.tangent().z()], [0.0, 1.0]);

        let end = registry
            .sample(edge, EdgeProgress::try_new(2.0).expect("valid progress"))
            .expect("end sample");
        assert_eq!(end.position(), points[2]);
        assert_eq!([end.tangent().x(), end.tangent().z()], [0.0, 1.0]);
    }

    #[test]
    fn single_segment_sampling_interpolates_position_by_core_ratio() {
        let lane_graph = graph(&[("A", 1.0, &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let points = [point(10.0, 0.0, 2.0), point(11.0, 0.0, 2.0)];
        let registry = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("valid registry");

        let pose = registry
            .sample(edge, EdgeProgress::try_new(0.25).expect("valid progress"))
            .expect("quarter sample");
        assert_eq!([pose.position().x(), pose.position().z()], [10.25, 2.0]);
        assert_eq!([pose.tangent().x(), pose.tangent().z()], [1.0, 0.0]);

        let foreign_graph = graph(&[("X", 1.0, &[]), ("Y", 1.0, &[])]);
        let unknown = foreign_graph.edge_handle("Y").expect("foreign edge");
        assert_eq!(
            registry
                .sample(unknown, EdgeProgress::ZERO)
                .expect_err("unknown handles fail before sampling"),
            SpatialError::UnknownEdgeHandle { edge: unknown }
        );
    }

    #[test]
    fn sample_checks_exact_core_range_and_basis_is_orthonormal() {
        let lane_graph = graph(&[("A", 2.0_f64.sqrt(), &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let points = [point(0.0, 0.0, 0.0), point(1.0, 1.0, 0.0)];
        let registry = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("valid registry");

        let pose = registry
            .sample(edge, EdgeProgress::try_new(0.5).expect("valid progress"))
            .expect("valid sample");
        let tangent = pose.tangent();
        let up = pose.up();
        let tangent_length = tangent.x().hypot(tangent.y()).hypot(tangent.z());
        let up_length = up.x().hypot(up.y()).hypot(up.z());
        let dot = tangent.x() * up.x() + tangent.y() * up.y() + tangent.z() * up.z();
        assert!((tangent_length - 1.0).abs() <= 2.0 * f32::EPSILON);
        assert!((up_length - 1.0).abs() <= 2.0 * f32::EPSILON);
        assert!(dot.abs() <= 2.0 * f32::EPSILON);

        let beyond = f64::from_bits((2.0_f64.sqrt()).to_bits() + 1);
        assert_eq!(
            registry
                .sample(
                    edge,
                    EdgeProgress::try_new(beyond).expect("non-negative progress")
                )
                .expect_err("progress is not silently snapped"),
            SpatialError::ProgressOutOfRange {
                edge,
                progress_meters: beyond,
                max_meters: 2.0_f64.sqrt(),
            }
        );
    }

    #[test]
    fn repeated_construction_and_sampling_preserve_value_bits() {
        let lane_graph = graph(&[("A", 2.0, &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let points = [
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(1.0, 0.0, 1.0),
        ];
        let progress = EdgeProgress::try_new(0.375).expect("valid progress");
        let first = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("first registry")
        .sample(edge, progress)
        .expect("first sample");
        let second = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("second registry")
        .sample(edge, progress)
        .expect("second sample");

        let bits = |pose: CanonicalPoseF32| {
            [
                pose.position().x().to_bits(),
                pose.position().y().to_bits(),
                pose.position().z().to_bits(),
                pose.tangent().x().to_bits(),
                pose.tangent().y().to_bits(),
                pose.tangent().z().to_bits(),
                pose.up().x().to_bits(),
                pose.up().y().to_bits(),
                pose.up().z().to_bits(),
            ]
        };
        assert_eq!(bits(first), bits(second));
    }

    #[test]
    fn registry_capacity_check_covers_u32_boundary_without_allocating() {
        assert_eq!(validate_registry_capacity(u32::MAX as usize), Ok(()));
        if usize::BITS > u32::BITS {
            let actual = u32::MAX as usize + 1;
            assert_eq!(
                validate_registry_capacity(actual),
                Err(SpatialError::RegistryCapacityExceeded {
                    actual,
                    max: u32::MAX as usize,
                })
            );
        }
    }

    #[test]
    #[ignore = "wall-clock diagnostic runs only with the fixed-machine evidence command"]
    fn private_lookup_and_slot_resolved_sampling_are_measured_separately() {
        const OPERATION_COUNT: usize = 100_000;
        const SAMPLE_COUNT: usize = 80;

        let lane_graph = graph(&[("A", 2.0, &[])]);
        let edge = lane_graph.edge_handle("A").expect("edge A");
        let points = [
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(1.0, 0.0, 1.0),
        ];
        let registry = SpatialRegistry::try_new(
            &lane_graph,
            frame_id(),
            [SpatialEdgeInput::new(edge, &points)],
        )
        .expect("valid diagnostic registry");
        let slot = registry.edge_slot(edge).expect("private edge slot");
        let progresses: Vec<_> = (0..OPERATION_COUNT)
            .map(|index| {
                EdgeProgress::try_new(2.0 * index as f64 / (OPERATION_COUNT - 1) as f64)
                    .expect("valid diagnostic progress")
            })
            .collect();

        let lookup_p95_ns = diagnostic_p95(SAMPLE_COUNT, || {
            for _ in 0..OPERATION_COUNT {
                black_box(registry.edge_slot(black_box(edge)));
            }
        });
        let resolved_p95_ns = diagnostic_p95(SAMPLE_COUNT, || {
            for progress in &progresses {
                black_box(registry.entries[slot as usize].sample(edge, *progress))
                    .expect("valid resolved sample");
            }
        });

        println!(
            "SPATIAL_PRIVATE_DIAGNOSTIC operations={OPERATION_COUNT} lookup_p95_ns={lookup_p95_ns} slot_resolved_p95_ns={resolved_p95_ns}"
        );
    }

    fn diagnostic_p95(sample_count: usize, mut operation: impl FnMut()) -> u128 {
        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let started = Instant::now();
            operation();
            samples.push(started.elapsed().as_nanos());
        }
        samples.sort_unstable();
        samples[(sample_count * 95).div_ceil(100) - 1]
    }
}
