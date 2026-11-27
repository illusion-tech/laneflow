//! 将几何已证的侧向净空下界换算为世界驾驶配置的通过速度上限。
use laneflow_static_contract::{MAX_SPEED_MM_S, MIN_SPEED_MM_S};
use thiserror::Error;

/// 每个世界的曲线最多保留八个节点，每个节点含两个 `u32`。
pub const MAX_CLEARANCE_SPEED_POINTS: usize = 8;

/// 一项显式驾驶配置。净空和速度分别使用毫米、毫米每秒。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearanceSpeedPoint {
    /// 整车与相关侧方实体的已证距离下界，单位毫米。
    pub clearance_mm: u32,
    /// 此余量允许的纵向通过速度上限，单位毫米每秒。
    pub speed_mm_s: u32,
}

/// 曲线定义不满足净空、速度域或宽裕端的要求。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ClearanceSpeedCurveError {
    /// 控制点数量不在二至八之间。
    #[error("侧向净空限速曲线的节点数 {count} 超出允许范围")]
    PointCount {
        /// 输入节点数。
        count: usize,
    },
    /// 节点速度超出道路机动车的正速度域。
    #[error("侧向净空限速曲线第 {index} 个节点的速度超出允许范围")]
    InvalidSpeed {
        /// 从零开始的节点下标。
        index: usize,
    },
    /// 节点的净空小于或等于前一节点。
    #[error("侧向净空限速曲线第 {index} 个节点的净空没有递增")]
    ClearanceOrder {
        /// 从零开始的节点下标。
        index: usize,
    },
    /// 节点速度小于前一节点。
    #[error("侧向净空限速曲线第 {index} 个节点的速度下降")]
    DecreasingSpeed {
        /// 从零开始的节点下标。
        index: usize,
    },
    /// 最后一个节点仍在限制正常道路速度。
    #[error("侧向净空限速曲线末点须达到规范速度域上限")]
    RestrictedFinalPoint,
}

/// 世界显式选择的单调分段线性曲线。控制点在构造时验证，查询无需分配。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearanceSpeedCurve {
    points: [ClearanceSpeedPoint; MAX_CLEARANCE_SPEED_POINTS],
    count: u8,
}

impl ClearanceSpeedCurve {
    /// 按输入顺序保留完整控制点，供试算及世界行为配置读回。
    ///
    /// # Errors
    ///
    /// 节点数须在二至八之间。净空严格递增，速度在规范正速度域内且不下降，
    /// 末点速度必须为最大规范速度。违反任一条件时拒绝整条曲线。
    pub fn try_new(points: Vec<ClearanceSpeedPoint>) -> Result<Self, ClearanceSpeedCurveError> {
        let count = points.len();
        if !(2..=MAX_CLEARANCE_SPEED_POINTS).contains(&count) {
            return Err(ClearanceSpeedCurveError::PointCount { count });
        }
        for (index, point) in points.iter().enumerate() {
            if !(MIN_SPEED_MM_S..=MAX_SPEED_MM_S).contains(&point.speed_mm_s) {
                return Err(ClearanceSpeedCurveError::InvalidSpeed { index });
            }
            if index > 0 {
                let previous = &points[index - 1];
                if point.clearance_mm <= previous.clearance_mm {
                    return Err(ClearanceSpeedCurveError::ClearanceOrder { index });
                }
                if point.speed_mm_s < previous.speed_mm_s {
                    return Err(ClearanceSpeedCurveError::DecreasingSpeed { index });
                }
            }
        }
        if points[count - 1].speed_mm_s != MAX_SPEED_MM_S {
            return Err(ClearanceSpeedCurveError::RestrictedFinalPoint);
        }
        // 尾部重复末点，保证相同有效曲线始终拥有相同的表示。
        let mut stored = [points[count - 1]; MAX_CLEARANCE_SPEED_POINTS];
        stored[..count].copy_from_slice(&points);
        Ok(Self {
            points: stored,
            count: count as u8,
        })
    }

    /// 按净空严格递增排列的已验证节点。
    #[must_use]
    pub fn points(&self) -> &[ClearanceSpeedPoint] {
        &self.points[..usize::from(self.count)]
    }

    /// 将已证的正侧向净空按毫米向下量化后查询。严格正但不足一毫米可传零。
    /// 调用前须已证明轨迹可通，接触和几何未证应由几何准入处理。
    /// 首点以下取首点速度，末点及以上返回规范最大速度。节点间向下取整到毫米每秒。
    #[must_use]
    pub fn speed_cap_mm_s(&self, proven_lower_mm: u32) -> u32 {
        let points = self.points();
        let upper = points.partition_point(|point| point.clearance_mm <= proven_lower_mm);
        let Some(right) = points.get(upper) else {
            return MAX_SPEED_MM_S;
        };
        if upper == 0 {
            return right.speed_mm_s;
        }
        let left = points[upper - 1];
        let offset = u64::from(proven_lower_mm - left.clearance_mm);
        let width = u64::from(right.clearance_mm - left.clearance_mm);
        let rise = u64::from(right.speed_mm_s - left.speed_mm_s);
        // offset < width ≤ u32::MAX，乘积可由 u64 精确表示，商不超过 rise。
        left.speed_mm_s + (offset * rise / width) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(clearance_mm: u32, speed_mm_s: u32) -> ClearanceSpeedPoint {
        ClearanceSpeedPoint {
            clearance_mm,
            speed_mm_s,
        }
    }

    #[test]
    fn explicit_curve_preserves_plateaus_and_rounds_each_segment_down() {
        let points = vec![
            point(50, 1_000),
            point(60, 1_000),
            point(63, 1_008),
            point(550, MAX_SPEED_MM_S),
        ];
        let curve = ClearanceSpeedCurve::try_new(points.clone()).unwrap();
        assert_eq!(curve.points(), points);
        for (clearance, speed) in [
            (0, 1_000),
            (50, 1_000),
            (55, 1_000),
            (60, 1_000),
            (61, 1_002),
            (62, 1_005),
            (63, 1_008),
            (549, 99_796),
            (550, MAX_SPEED_MM_S),
            (u32::MAX, MAX_SPEED_MM_S),
        ] {
            assert_eq!(curve.speed_cap_mm_s(clearance), speed);
        }
        let widest = ClearanceSpeedCurve::try_new(vec![
            point(0, MIN_SPEED_MM_S),
            point(u32::MAX, MAX_SPEED_MM_S),
        ])
        .unwrap();
        assert_eq!(widest.speed_cap_mm_s(u32::MAX - 1), MAX_SPEED_MM_S - 1);
    }

    #[test]
    fn rejects_invalid_curves_before_use() {
        use ClearanceSpeedCurveError::*;
        let first = point(1, 100);
        let last = point(3, MAX_SPEED_MM_S);
        for (points, error) in [
            (vec![], PointCount { count: 0 }),
            (vec![last], PointCount { count: 1 }),
            (
                vec![first; MAX_CLEARANCE_SPEED_POINTS + 1],
                PointCount {
                    count: MAX_CLEARANCE_SPEED_POINTS + 1,
                },
            ),
            (vec![point(0, 0), last], InvalidSpeed { index: 0 }),
            (
                vec![first, point(3, MAX_SPEED_MM_S + 1)],
                InvalidSpeed { index: 1 },
            ),
            (vec![first, first, last], ClearanceOrder { index: 1 }),
            (
                vec![first, point(0, 100), last],
                ClearanceOrder { index: 1 },
            ),
            (
                vec![first, point(2, 99), last],
                DecreasingSpeed { index: 1 },
            ),
            (vec![first, point(2, 101)], RestrictedFinalPoint),
        ] {
            assert_eq!(ClearanceSpeedCurve::try_new(points), Err(error));
        }
        let points = (0..MAX_CLEARANCE_SPEED_POINTS)
            .map(|i| point(i as u32, MAX_SPEED_MM_S))
            .collect();
        assert!(ClearanceSpeedCurve::try_new(points).is_ok());
    }
}
