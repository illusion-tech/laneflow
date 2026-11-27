use laneflow_static_network::{StationAddress, vehicle::PoseBounds};

use super::*;

/// 与一条普通输入同拍的横移观测。调用方只为正在横移的记录提供该项。
pub struct HandoffPoseInput<'a> {
    /// 被替换的输入数组下标，各项须严格递增。
    pub input_index: usize,
    /// 求值所用的路网修订。
    pub revision: NetworkRevisionId,
    /// 车体参考姿态的规范框架。
    pub frame: CanonicalFrameOrdinal,
    /// 同一次交通求值的后轴和车体三轴。
    pub pose: &'a PoseBounds,
    /// 同一次交通求值的前杠边和进度范围。缺少已证位置时整批失败。
    pub front: Option<StationAddress>,
    /// 前杠在所属路面上的投影，按框架 X/Y/Z 排列，单位毫米。
    pub front_ground_mm: Option<[[i64; 2]; 3]>,
}

impl SpatialSession {
    pub(super) fn sample_handoff(
        &self,
        source: PoseSource,
        handoff: HandoffPoseInput<'_>,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        let PoseSource::Lane { edge, progress_mm } = source else {
            return Err(SpatialError::InvalidHandoffPose);
        };
        let Some(front) = handoff.front else {
            return Err(SpatialError::InvalidHandoffPose);
        };
        if handoff.revision != self.network_revision()
            || front.edge != edge
            || !(front.progress_mm[0]..=front.progress_mm[1]).contains(&progress_mm)
        {
            return Err(SpatialError::InvalidHandoffPose);
        }
        let (geometry, _) = self.lane_geometry(edge, progress_mm)?;
        if handoff.frame != geometry.canonical_frame() {
            return Err(SpatialError::BatchFrameMismatch {
                expected_frame: geometry.canonical_frame(),
                actual_frame: handoff.frame,
            });
        }
        let mut position = [0.0; 3];
        let bounds = handoff
            .front_ground_mm
            .ok_or(SpatialError::InvalidHandoffPose)?;
        for (component, [lo, hi]) in position.iter_mut().zip(bounds) {
            if lo > hi {
                return Err(SpatialError::InvalidHandoffPose);
            }
            *component = ((lo as f64).midpoint(hi as f64) / 1_000.) as f32;
        }
        let position = CanonicalPoint3F32::try_new(position[0], position[1], position[2])?;
        let axes = [handoff.pose.forward, handoff.pose.left, handoff.pose.up];
        for i in 0..3 {
            if !dot_encloses(axes[i], axes[i], 1.) || !dot_encloses(axes[i], axes[(i + 1) % 3], 0.)
            {
                return Err(SpatialError::InvalidHandoffPose);
            }
        }
        let tangent = nominal_direction(handoff.pose.forward)?;
        let left = nominal_direction(handoff.pose.left)?;
        let expected_up = nominal_direction(handoff.pose.up)?;
        let up = cross(tangent, left).try_normalize()?;
        if up.x() * expected_up.x() + up.y() * expected_up.y() + up.z() * expected_up.z() <= 0. {
            return Err(SpatialError::InvalidHandoffPose);
        }
        Ok((
            handoff.frame,
            CanonicalPoseF32::from_parts(position, tangent, up),
        ))
    }
}

// 三轴的分量存在舍入与来源误差。只有区间排除了单位长度或正交时才拒绝；
// 各标量乘加向外取整，避免用人为角度容差替代交通证据。
fn dot_encloses(a: [[f64; 2]; 3], b: [[f64; 2]; 3], target: f64) -> bool {
    let (mut lo, mut hi) = (0.0, 0.0);
    for ([a0, a1], [b0, b1]) in a.into_iter().zip(b) {
        let products = [a0 * b0, a0 * b1, a1 * b0, a1 * b1];
        let min = products.into_iter().fold(f64::INFINITY, f64::min);
        let max = products.into_iter().fold(f64::NEG_INFINITY, f64::max);
        lo = (lo + min.next_down()).next_down();
        hi = (hi + max.next_up()).next_up();
    }
    lo <= target && target <= hi
}

fn nominal_direction(bounds: [[f64; 2]; 3]) -> Result<CanonicalUnitVector3F32, SpatialError> {
    if !bounds
        .iter()
        .all(|&[lo, hi]| lo.is_finite() && hi.is_finite() && lo <= hi && lo <= 1. && hi >= -1.)
        || bounds.iter().all(|&[lo, hi]| lo <= 0. && hi >= 0.)
    {
        return Err(SpatialError::InvalidHandoffPose);
    }
    unit_from_array(bounds.map(|[lo, hi]| lo.midpoint(hi) as f32))
}
