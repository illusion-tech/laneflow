use std::sync::Arc;

use laneflow_static_contract::{
    CanonicalFrameOrdinal, LaneEdgeOrdinal, NetworkRevisionId, ParkingSpaceOrdinal,
};
use laneflow_static_network::{LaneGeometryView, SharedNetworkRevision};

use crate::{
    CanonicalPoint3F32, CanonicalPoseF32, CanonicalUnitVector3F32, CanonicalVector3F32,
    FramePlacementToken, PoseRecordId, SpatialError,
};

/// 共享根上的位姿来源。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoseSource {
    /// 车道进度。
    Lane {
        /// 共享根边序号。
        edge: LaneEdgeOrdinal,
        /// 当前边上的毫米进度。
        progress_mm: u32,
    },
    /// 停车位。
    Parking {
        /// 共享根停车位序号。
        space: ParkingSpaceOrdinal,
    },
}

/// 一条 pose 批次输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseInput {
    record: PoseRecordId,
    source: PoseSource,
}

impl PoseInput {
    /// 车道采样输入。
    #[must_use]
    pub const fn lane(record: PoseRecordId, edge: LaneEdgeOrdinal, progress_mm: u32) -> Self {
        Self {
            record,
            source: PoseSource::Lane { edge, progress_mm },
        }
    }

    /// 停车位采样输入。
    #[must_use]
    pub const fn parking(record: PoseRecordId, space: ParkingSpaceOrdinal) -> Self {
        Self {
            record,
            source: PoseSource::Parking { space },
        }
    }

    /// 调用方分配的不透明 pose 记录身份。
    #[must_use]
    pub const fn record(self) -> PoseRecordId {
        self.record
    }

    /// 位姿来源。
    #[must_use]
    pub const fn source(self) -> PoseSource {
        self.source
    }

    /// 由记录身份与来源构造。
    #[must_use]
    pub const fn from_source(record: PoseRecordId, source: PoseSource) -> Self {
        Self { record, source }
    }
}

/// `SpatialSession::bind` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpatialBindError {
    /// 有 Spatial component 但缺少 lane-pose，不能做车辆位姿采样。
    MissingLanePose,
}

/// 绑定到一根 `SharedNetworkRevision` 的 Spatial session。
pub struct SpatialSession {
    revision: Arc<SharedNetworkRevision>,
    scratch: Vec<CanonicalPoseRecord>,
}

impl SpatialSession {
    /// 绑定根 `Arc`。无 Spatial 时返回 `Ok(None)`。有 Spatial 但无 lane-pose 时失败。
    ///
    /// # Errors
    ///
    /// 根不含 Spatial 包时返回 `Ok(None)`（headless 根，不是错误）；存在
    /// Spatial 包但缺少 lane-pose 数据时返回
    /// [`SpatialBindError::MissingLanePose`]。
    pub fn bind(revision: Arc<SharedNetworkRevision>) -> Result<Option<Self>, SpatialBindError> {
        match revision.spatial() {
            None => Ok(None),
            Some(spatial) if spatial.lane_pose().is_none() => {
                Err(SpatialBindError::MissingLanePose)
            }
            Some(_) => Ok(Some(Self {
                revision,
                scratch: Vec::new(),
            })),
        }
    }

    /// 绑定所用根。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        Arc::clone(&self.revision)
    }

    /// 根修订身份。
    #[must_use]
    pub fn network_revision(&self) -> NetworkRevisionId {
        self.revision.network_revision()
    }

    /// 按调用方顺序提取 pose 批次。
    /// `placement_token` 原样回显。
    ///
    /// # Errors
    ///
    /// 任一记录共享位姿提取失败（包装为 [`SpatialError::SharedPoseRecordFailed`]）
    /// 或同一批混用多个 canonical frame（[`SpatialError::BatchFrameMismatch`]）时返回；
    /// 整批失败且不改 `output`。
    pub fn extract_pose_batch(
        &mut self,
        placement_token: FramePlacementToken,
        inputs: &[PoseInput],
        output: &mut CanonicalPoseBatch,
    ) -> Result<(), SpatialError> {
        #[cfg(feature = "pose-profiling")]
        let sample_started = std::time::Instant::now();
        self.scratch.clear();
        self.scratch.reserve(inputs.len());
        let mut frame: Option<CanonicalFrameOrdinal> = None;
        for (input_index, input) in inputs.iter().copied().enumerate() {
            let (sampled_frame, pose) = match self.sample(input.source) {
                Ok(sampled) => sampled,
                Err(source) => {
                    self.scratch.clear();
                    return Err(SpatialError::SharedPoseRecordFailed {
                        input_index,
                        record: input.record,
                        source: Box::new(source),
                    });
                }
            };
            if let Some(expected) = frame {
                if expected != sampled_frame {
                    self.scratch.clear();
                    return Err(SpatialError::BatchFrameMismatch {
                        expected_frame: expected,
                        actual_frame: sampled_frame,
                    });
                }
            } else {
                frame = Some(sampled_frame);
            }
            self.scratch.push(CanonicalPoseRecord {
                record: input.record,
                pose,
            });
        }
        let network_revision = Some(self.network_revision());

        // 全部采样和 frame 检查已成功；提交阶段不再执行可恢复失败操作。
        #[cfg(feature = "pose-profiling")]
        let sample_ns = sample_started.elapsed().as_nanos();
        #[cfg(feature = "pose-profiling")]
        let commit_started = std::time::Instant::now();
        std::mem::swap(&mut self.scratch, &mut output.records);

        // 接管上一批输出的存储，保留容量供下一批候选复用。
        self.scratch.clear();

        output.network_revision = network_revision;
        output.canonical_frame = frame;
        output.placement_token = placement_token;
        #[cfg(feature = "pose-profiling")]
        {
            use std::io::Write as _;
            let commit_ns = commit_started.elapsed().as_nanos();
            let _ = writeln!(
                std::io::stderr().lock(),
                "pose-spatial sample_ns={sample_ns} commit_ns={commit_ns} scratch_len={} scratch_cap={} output_len={} output_cap={} record_size={} logical_copy_bytes=0",
                self.scratch.len(),
                self.scratch.capacity(),
                output.records.len(),
                output.records.capacity(),
                std::mem::size_of::<CanonicalPoseRecord>()
            );
        }

        Ok(())
    }

    fn sample(
        &self,
        source: PoseSource,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        match source {
            PoseSource::Lane { edge, progress_mm } => self.sample_lane(edge, progress_mm),
            PoseSource::Parking { space } => self.sample_parking(space),
        }
    }

    fn sample_lane(
        &self,
        edge: LaneEdgeOrdinal,
        progress_mm: u32,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        let traffic_length_mm = *self
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .get(edge.index())
            .ok_or(SpatialError::UnknownLaneEdge { edge })?;
        if progress_mm > traffic_length_mm {
            return Err(SpatialError::SharedProgressOutOfRange {
                edge,
                progress_mm,
                length_mm: traffic_length_mm,
            });
        }
        let geometry = self
            .revision
            .spatial()
            .and_then(|spatial| spatial.lane_pose())
            .and_then(|network| network.lane_geometry(edge))
            .ok_or(SpatialError::UnknownLaneEdge { edge })?;
        let pose = sample_lane_geometry(geometry, traffic_length_mm, progress_mm)?;
        Ok((geometry.canonical_frame(), pose))
    }

    fn sample_parking(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        let relations = self.revision.traffic().relations();
        let (entry_edge, entry_progress) = relations
            .parking_space_entry(space)
            .ok_or(SpatialError::UnknownParkingSpace { space })?;
        let (lateral, heading, _length, _width) = relations
            .parking_space_geometry(space)
            .ok_or(SpatialError::UnknownParkingSpace { space })?;
        let (frame, anchor) = self.sample_lane(entry_edge, entry_progress)?;
        let left = cross(anchor.up(), anchor.tangent())
            .try_normalize()
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking left basis",
                source: Box::new(source),
            })?;
        let lateral = (f64::from(lateral) / 1_000.0) as f32;
        if !lateral.is_finite() {
            return Err(SpatialError::UnknownParkingSpace { space });
        }
        let displacement = left.as_vector().checked_scale(lateral).map_err(|source| {
            SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking position",
                source: Box::new(source),
            }
        })?;
        let position = anchor
            .position()
            .checked_add_vector(displacement)
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking position",
                source: Box::new(source),
            })?;
        let (sin_heading, cos_heading) = heading.sin_cos();
        let forward = anchor
            .tangent()
            .as_vector()
            .checked_scale(cos_heading)
            .and_then(|forward| {
                left.as_vector()
                    .checked_scale(sin_heading)
                    .and_then(|lateral| forward.checked_add(lateral))
            })
            .and_then(CanonicalVector3F32::try_normalize)
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking heading",
                source: Box::new(source),
            })?;
        Ok((
            frame,
            CanonicalPoseF32::from_parts(position, forward, anchor.up()),
        ))
    }
}

/// 共享根 pose 批次。
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalPoseBatch {
    network_revision: Option<NetworkRevisionId>,
    canonical_frame: Option<CanonicalFrameOrdinal>,
    placement_token: FramePlacementToken,
    records: Vec<CanonicalPoseRecord>,
}

impl Default for CanonicalPoseBatch {
    fn default() -> Self {
        Self {
            network_revision: None,
            canonical_frame: None,
            placement_token: FramePlacementToken::new(0),
            records: Vec::new(),
        }
    }
}

impl CanonicalPoseBatch {
    /// 空批次。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 采样批次来源的网络修订；`None` 仅出现在批次尚未填充时（headless 修订
    /// 不会产生本批次）。
    #[must_use]
    pub const fn network_revision(&self) -> Option<NetworkRevisionId> {
        self.network_revision
    }

    /// 采样批次的规范坐标框架；尚未填充，或成功提取的批次为空时，为 `None`。
    #[must_use]
    pub const fn canonical_frame(&self) -> Option<CanonicalFrameOrdinal> {
        self.canonical_frame
    }

    /// 提取时调用方提供的放置令牌原样回显。
    #[must_use]
    pub const fn placement_token(&self) -> FramePlacementToken {
        self.placement_token
    }

    /// 批内记录切片；记录身份为调用方分配的不透明值，按提交顺序排列。
    #[must_use]
    pub fn records(&self) -> &[CanonicalPoseRecord] {
        &self.records
    }
}

/// 单条共享根 pose 记录。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoseRecord {
    record: PoseRecordId,
    pose: CanonicalPoseF32,
}

impl CanonicalPoseRecord {
    /// 本记录的调用方分配身份（提取时原样回显）。
    #[must_use]
    pub const fn record(self) -> PoseRecordId {
        self.record
    }

    /// 本记录的规范位姿。
    #[must_use]
    pub const fn pose(self) -> CanonicalPoseF32 {
        self.pose
    }
}

fn sample_lane_geometry(
    geometry: LaneGeometryView<'_>,
    traffic_length_mm: u32,
    progress_mm: u32,
) -> Result<CanonicalPoseF32, SpatialError> {
    let points = geometry.points();
    let segments = geometry.segments();
    if points.len() < 2 || segments.is_empty() {
        return Err(SpatialError::UnknownLaneEdge {
            edge: LaneEdgeOrdinal::from_raw(0),
        });
    }
    let pose_at =
        |point_index: usize, segment_index: usize| -> Result<CanonicalPoseF32, SpatialError> {
            let point = points[point_index];
            let segment = segments[segment_index];
            Ok(CanonicalPoseF32::from_parts(
                CanonicalPoint3F32::try_new(point.x, point.y, point.z)?,
                unit_from_array(segment.tangent)?,
                unit_from_array(segment.up)?,
            ))
        };
    if progress_mm == 0 {
        return pose_at(0, 0);
    }
    if progress_mm == traffic_length_mm {
        return pose_at(points.len() - 1, segments.len() - 1);
    }
    let arc = geometry.arc_length_meters();
    let geometry_s =
        (f64::from(progress_mm) / f64::from(traffic_length_mm) * f64::from(arc)) as f32;
    if geometry_s >= arc {
        return pose_at(points.len() - 1, segments.len() - 1);
    }
    let segment_index = segments
        .partition_point(|segment| segment.cumulative_end_meters <= geometry_s)
        .min(segments.len() - 1);
    let segment = segments[segment_index];
    let start_s = if segment_index == 0 {
        0.0
    } else {
        segments[segment_index - 1].cumulative_end_meters
    };
    let ratio = (geometry_s - start_s) / segment.length_meters;
    let start = points[segment_index];
    let end = points[segment_index + 1];
    let position = CanonicalPoint3F32::try_new(
        start.x + (end.x - start.x) * ratio,
        start.y + (end.y - start.y) * ratio,
        start.z + (end.z - start.z) * ratio,
    )?;
    Ok(CanonicalPoseF32::from_parts(
        position,
        unit_from_array(segment.tangent)?,
        unit_from_array(segment.up)?,
    ))
}

fn unit_from_array(values: [f32; 3]) -> Result<CanonicalUnitVector3F32, SpatialError> {
    CanonicalVector3F32::try_new(values[0], values[1], values[2])?.try_normalize()
}

fn cross(left: CanonicalUnitVector3F32, right: CanonicalUnitVector3F32) -> CanonicalVector3F32 {
    CanonicalVector3F32::try_new(
        left.y() * right.z() - left.z() * right.y(),
        left.z() * right.x() - left.x() * right.z(),
        left.x() * right.y() - left.y() * right.x(),
    )
    .expect("crossing finite unit directions produces a finite vector")
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );
    const EDGE_A0: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(0);

    fn test_revision() -> Arc<SharedNetworkRevision> {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision")
    }

    fn bound_session() -> SpatialSession {
        SpatialSession::bind(test_revision())
            .expect("bind")
            .expect("session")
    }

    fn edge_a0_length_mm() -> u32 {
        test_revision().traffic().lane_lengths_millimetres()[EDGE_A0.index()]
    }

    /// 生成 `count` 条 frame 0 内的合法车道输入；进度在边长内确定性变化。
    fn lane_inputs(count: usize) -> Vec<PoseInput> {
        let length_mm = edge_a0_length_mm();
        (0..count)
            .map(|index| {
                let record = u32::try_from(index).expect("test record id fits u32");
                PoseInput::lane(
                    PoseRecordId::new(record),
                    EDGE_A0,
                    (record.wrapping_mul(37)) % length_mm,
                )
            })
            .collect()
    }

    /// 一条进度越界的非法输入，用于触发采样失败。
    fn bad_input(record: u32) -> PoseInput {
        PoseInput::lane(PoseRecordId::new(record), EDGE_A0, edge_a0_length_mm() + 1)
    }

    fn token(value: u64) -> FramePlacementToken {
        FramePlacementToken::new(value)
    }

    /// 指针级证明：两侧 backing 足够容纳本批时，成功提交交换 Vec 所有权而非复制。
    /// 指针只是内部存储机制的测试证据，不是公开 API 保证。
    #[test]
    fn successful_commit_swaps_backing_ownership() {
        let inputs = lane_inputs(8);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();
        // 暖机两侧，使被验证区间内不会发生扩容。
        session
            .extract_pose_batch(token(1), &inputs, &mut output)
            .expect("warm-up 1");
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("warm-up 2");
        assert!(session.scratch.capacity() >= inputs.len());
        assert!(output.records.capacity() >= inputs.len());

        let scratch_ptr = session.scratch.as_ptr();
        let scratch_cap = session.scratch.capacity();
        let output_ptr = output.records.as_ptr();
        let output_cap = output.records.capacity();

        session
            .extract_pose_batch(token(3), &inputs, &mut output)
            .expect("measured call");

        assert_eq!(
            output.records.as_ptr(),
            scratch_ptr,
            "output takes scratch backing"
        );
        assert_eq!(output.records.capacity(), scratch_cap);
        assert_eq!(output.records.len(), inputs.len());
        assert_eq!(
            session.scratch.as_ptr(),
            output_ptr,
            "scratch takes old output backing"
        );
        assert_eq!(session.scratch.capacity(), output_cap);
        assert!(session.scratch.is_empty());
    }

    /// B01：新 Session 与新 output 从零开始，前两次调用发生增长，随后进入稳定状态。
    #[test]
    fn b01_new_session_and_output_reach_stable_capacity() {
        let inputs = lane_inputs(8);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();

        // 第一次调用：scratch 增长后交出，接管回来的是空 output backing。
        session
            .extract_pose_batch(token(1), &inputs, &mut output)
            .expect("first call");
        assert!(output.records.capacity() >= inputs.len());
        assert_eq!(
            session.scratch.capacity(),
            0,
            "session takes the fresh output's empty backing"
        );

        // 第二次调用：另一侧从零增长，此后两侧容量均足够。
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("second call");
        assert!(session.scratch.capacity() >= inputs.len());
        assert!(output.records.capacity() >= inputs.len());

        // 稳定状态：容量集合不再变化，只随所有权轮换交换归属。
        let stable = (session.scratch.capacity(), output.records.capacity());
        session
            .extract_pose_batch(token(3), &inputs, &mut output)
            .expect("third call");
        assert_eq!(
            (output.records.capacity(), session.scratch.capacity()),
            stable,
            "capacities rotate with ownership"
        );
        session
            .extract_pose_batch(token(4), &inputs, &mut output)
            .expect("fourth call");
        assert_eq!(
            (session.scratch.capacity(), output.records.capacity()),
            stable,
            "capacities return after a full rotation"
        );
    }

    /// B02：两侧暖机后，同一 Session/output 固定规模复用不再增长（无新增分配）。
    #[test]
    fn b02_fixed_size_reuse_has_no_growth_once_warm() {
        let inputs = lane_inputs(16);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(1), &inputs, &mut output)
            .expect("warm-up 1");
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("warm-up 2");

        let scratch_ptr = session.scratch.as_ptr();
        let scratch_cap = session.scratch.capacity();
        let output_ptr = output.records.as_ptr();
        let output_cap = output.records.capacity();
        assert!(scratch_cap >= inputs.len());
        assert!(output_cap >= inputs.len());

        // 偶数次调用后 backing 回到原侧；指针与容量均不变说明没有扩容或重分配。
        for round in 0..6 {
            session
                .extract_pose_batch(token(10 + round), &inputs, &mut output)
                .expect("steady call");
            assert_eq!(output.records.len(), inputs.len());
        }
        assert_eq!(session.scratch.as_ptr(), scratch_ptr);
        assert_eq!(session.scratch.capacity(), scratch_cap);
        assert_eq!(session.scratch.len(), 0);
        assert_eq!(output.records.as_ptr(), output_ptr);
        assert_eq!(output.records.capacity(), output_cap);
    }

    /// B03：规模增长后再缩小，扩容有据可查；缩小时不释放 backing，旧尾部不可见。
    #[test]
    fn b03_shrinking_batch_keeps_capacity_and_hides_tail() {
        let big = lane_inputs(64);
        let small = lane_inputs(4);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(1), &big, &mut output)
            .expect("big batch 1");
        session
            .extract_pose_batch(token(2), &big, &mut output)
            .expect("big batch 2");
        assert!(output.records.capacity() >= big.len());
        let grown_capacity = output.records.capacity();

        session
            .extract_pose_batch(token(3), &small, &mut output)
            .expect("small batch");
        assert_eq!(output.records.len(), small.len());
        assert!(
            output.records.capacity() >= grown_capacity,
            "shrinking batch keeps the grown backing"
        );
        assert_eq!(
            output
                .records()
                .iter()
                .map(|record| record.record().raw())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "old tail records stay invisible"
        );
    }

    /// B04：两个不同容量的 output 交替使用时，容量跟随 backing 轮换，
    /// Session 侧容量可以回落，不能假定单调不减。
    #[test]
    fn b04_alternating_outputs_rotate_capacity_with_backing() {
        let big = lane_inputs(64);
        let small = lane_inputs(8);
        let mut session = bound_session();
        let mut a = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(1), &big, &mut a)
            .expect("warm a 1");
        session
            .extract_pose_batch(token(2), &big, &mut a)
            .expect("warm a 2");
        let scratch_backing = (session.scratch.as_ptr(), session.scratch.capacity());
        assert!(scratch_backing.1 >= big.len());

        // 小 output 首次更新：直接接住 Session 侧的大 backing，自身无需增长。
        let mut b = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(3), &small, &mut b)
            .expect("fill b");
        assert_eq!(b.records.as_ptr(), scratch_backing.0);
        assert_eq!(b.records.capacity(), scratch_backing.1);
        assert_eq!(b.records.len(), small.len());
        assert_eq!(
            session.scratch.capacity(),
            0,
            "session falls back to b's previous empty backing"
        );

        // 再更新大 output：Session 侧从零重新增长，证明分配由轮换决定。
        session
            .extract_pose_batch(token(4), &big, &mut a)
            .expect("update a");
        assert!(session.scratch.capacity() >= big.len());
        assert!(a.records.capacity() >= big.len());
        assert_eq!(a.records.len(), big.len());
        assert!(b.records.capacity() >= big.len());
        assert_eq!(b.records.len(), small.len());
    }

    /// B05：已预热 Session 改用全新 output 时，如实记录新 backing 进入轮换的扩容。
    #[test]
    fn b05_warm_session_with_fresh_output_reallocates_scratch() {
        let inputs = lane_inputs(16);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(1), &inputs, &mut output)
            .expect("warm-up 1");
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("warm-up 2");
        let warm_backing = (session.scratch.as_ptr(), session.scratch.capacity());

        // 全新 output 第一次更新接住暖 backing；Session 接回空 backing。
        let mut fresh = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(3), &inputs, &mut fresh)
            .expect("fresh output first call");
        assert_eq!(fresh.records.as_ptr(), warm_backing.0);
        assert_eq!(fresh.records.capacity(), warm_backing.1);
        assert_eq!(session.scratch.capacity(), 0);

        // 第二次调用必须重新增长 Session 侧。
        session
            .extract_pose_batch(token(4), &inputs, &mut fresh)
            .expect("fresh output second call");
        assert!(session.scratch.capacity() >= inputs.len());
        assert!(fresh.records.capacity() >= inputs.len());
        assert_eq!(fresh.records.len(), inputs.len());
    }

    /// B06：大批次、空批次、小批次、空批次交替时，所有权轮换与元数据始终正确。
    #[test]
    fn b06_big_empty_small_empty_batches_rotate_cleanly() {
        let big = lane_inputs(32);
        let small = lane_inputs(4);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();

        session
            .extract_pose_batch(token(1), &big, &mut output)
            .expect("big");
        assert_eq!(output.records.len(), 32);
        assert!(output.canonical_frame().is_some());
        assert!(session.scratch.is_empty());

        session
            .extract_pose_batch(token(2), &[], &mut output)
            .expect("empty");
        assert!(output.records.is_empty());
        assert_eq!(output.canonical_frame(), None);
        assert_eq!(output.network_revision(), Some(session.network_revision()));
        assert_eq!(output.placement_token(), token(2));
        assert!(session.scratch.is_empty());

        session
            .extract_pose_batch(token(3), &small, &mut output)
            .expect("small");
        assert_eq!(output.records.len(), 4);
        assert!(output.canonical_frame().is_some());
        assert_eq!(output.placement_token(), token(3));

        session
            .extract_pose_batch(token(4), &[], &mut output)
            .expect("empty again");
        assert!(output.records.is_empty());
        assert_eq!(output.canonical_frame(), None);
        assert_eq!(output.placement_token(), token(4));
        assert!(session.scratch.is_empty());
    }

    /// B07：失败不交换也不破坏旧 output backing；scratch 清空后可继续复用。
    #[test]
    fn b07_failure_keeps_output_backing_and_scratch_reusable() {
        let inputs = lane_inputs(16);
        let mut session = bound_session();
        let mut output = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(1), &inputs, &mut output)
            .expect("warm-up 1");
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("warm-up 2");

        let before = output.clone();
        let output_ptr = output.records.as_ptr();
        let output_cap = output.records.capacity();

        let mut failing = inputs.clone();
        failing[7] = bad_input(999);
        let error = session
            .extract_pose_batch(token(3), &failing, &mut output)
            .expect_err("batch must fail");
        assert!(matches!(
            error,
            SpatialError::SharedPoseRecordFailed { input_index: 7, .. }
        ));
        assert_eq!(output, before, "failure keeps the full old output");
        assert_eq!(output.records.as_ptr(), output_ptr);
        assert_eq!(output.records.capacity(), output_cap);
        assert!(session.scratch.is_empty());
        assert!(
            session.scratch.capacity() >= inputs.len(),
            "failure clears scratch but keeps its capacity"
        );

        // 失败后同一 output 重试成功，正常完成所有权轮换。
        session
            .extract_pose_batch(token(4), &inputs, &mut output)
            .expect("retry");
        assert_eq!(output.records.len(), inputs.len());
        assert_eq!(output.placement_token(), token(4));
        assert!(session.scratch.is_empty());
    }
}
