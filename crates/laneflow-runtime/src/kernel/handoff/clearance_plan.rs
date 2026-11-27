use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::num::NonZeroU32;

use laneflow_static_contract::MAX_SPEED_MM_S;
use laneflow_static_network::vehicle::{Failure, HandoffOutcome};
use thiserror::Error;

use super::{BoundRouteHandoff, ClearanceSpeedCurve, RouteHandoffTrial, RouteHandoffTrialError};
use crate::TrafficWorld;

/// 动态换道的显式驾驶配置。拍间试算从调用方读取；两个尺度都参与行为重建。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffDrivingConfig {
    /// 从已证绝对侧向余量换算速度上限。
    pub curve: ClearanceSpeedCurve,
    /// 每个侧向几何段的最大后轴参数跨度（毫米）。
    pub max_span_mm: NonZeroU32,
    /// 同一候选累计三维行程求界的最大宽度（毫米）。
    pub travel_tolerance_mm: NonZeroU32,
}

/// 后轴实际累计行程中须遵守同一侧向限速的闭区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearanceSpeedWindow {
    /// 受限的整数毫米格，含首末两格；格 m 覆盖 [m, m+1) 毫米。
    pub travel_mm: [u64; 2],
    /// 此范围的最大通过速度（毫米/秒）。
    pub speed_mm_s: u32,
}

/// 某次已绑定候选的净空限速计划。仅在同一修订、候选、车型、准入速度、
/// 驾驶配置与固定步长上复用。改变其中一项后须重新准备计划。每拍的交通许可另行裁决。
#[derive(Clone, Debug, PartialEq)]
pub struct RouteHandoffSpeedPlan {
    pub(super) windows: Vec<ClearanceSpeedWindow>,
    pub(super) length_lower_um: u64,
    pub(super) admitted_speed_mm_s: u32,
    pub(super) comfort_decel: f32,
    pub(super) fixed_delta_ms: u32,
    /// 原候选的轮迹、转向、扫过、过缝与两侧准入工作合计。整条路线的静态权限检查另计。
    pub admission_work: u32,
    /// 实际完成的侧向净空工作数，另于原候选准入的工作预算。
    pub clearance_work: u32,
    /// 全部限速变更位置及终点共用一次累计行程查询的工作数。
    pub travel_work: u32,
}

impl RouteHandoffSpeedPlan {
    /// 已取重叠最小上限、按行程递增且互不交叠的局部限制。
    #[must_use]
    pub fn windows(&self) -> &[ClearanceSpeedWindow] {
        &self.windows
    }

    /// 可确认尚在候选上的最大后轴累计行程，单位毫米。
    #[must_use]
    pub fn length_lower_mm(&self) -> u64 {
        self.length_lower_um / 1000
    }
}

/// 拍间准备完整限速计划的失败原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffSpeedPlanError {
    /// 路线、窗口、车型或通行规则不符。
    #[error(transparent)]
    Trial(#[from] RouteHandoffTrialError),
    /// 原候选的整车、转向或交接尚未准入。
    #[error("候选没有可用的交接轨迹：{0:?}")]
    Unlocated(HandoffOutcome),
    /// 无法取得完整、有序的侧向净空覆盖。
    #[error("侧向净空未证：{reason:?}，候选区间 {range_mm:?}，工作 {work}")]
    Clearance {
        /// 几何失败原因。
        reason: Failure,
        /// 首个未证的有向候选参数区间（毫米）。
        range_mm: Option<[u64; 2]>,
        /// 已消耗的侧向求证工作。
        work: u32,
    },
    /// 无法把全部限速变化与终点定位到真实三维行程。
    #[error("限速位置的后轴行程未证：{reason:?}，工作 {work}")]
    Travel {
        /// 几何失败原因。
        reason: Failure,
        /// 已消耗的累计行程工作。
        work: u32,
    },
    /// 准备区间或归并工作区时无法预留空间。
    #[error("无法预留侧向限速计划空间")]
    Allocation,
}

// 相同上限的连续段共享一次弧长边界。返回 (结束参数) 和对应区间的上限。
fn speed_runs(
    segments: impl IntoIterator<Item = (u64, Option<u32>)>,
    curve: &ClearanceSpeedCurve,
) -> Result<(Vec<u64>, Vec<u32>), RouteHandoffSpeedPlanError> {
    let mut ends = Vec::new();
    let mut speeds = Vec::new();
    for (end_mm, clearance_mm) in segments {
        let speed = clearance_mm.map_or(MAX_SPEED_MM_S, |mm| curve.speed_cap_mm_s(mm));
        if speeds.last() == Some(&speed)
            && let Some(last) = ends.last_mut()
        {
            *last = end_mm;
        } else {
            ends.try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            speeds
                .try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            ends.push(end_mm);
            speeds.push(speed);
        }
    }
    Ok((ends, speeds))
}

// 边界求界可互相重叠。按闭区间取全部有效上限的最小值，解除时刻是末端格的下一毫米。
fn minimum_envelope(
    mut spans: Vec<ClearanceSpeedWindow>,
) -> Result<Vec<ClearanceSpeedWindow>, RouteHandoffSpeedPlanError> {
    spans.sort_unstable_by_key(|w| w.travel_mm[0]);
    let Some(first) = spans.first() else {
        return Ok(spans);
    };
    let mut at = first.travel_mm[0];
    let mut starts = spans.iter().peekable();
    let mut active = BinaryHeap::new();
    active
        .try_reserve(spans.len())
        .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
    let mut out: Vec<ClearanceSpeedWindow> = Vec::new();
    loop {
        while let Some(w) = starts.next_if(|w| w.travel_mm[0] <= at) {
            active.push(Reverse((w.speed_mm_s, w.travel_mm[1])));
        }
        while active.peek().is_some_and(|&Reverse((_, end))| end < at) {
            active.pop();
        }
        let next_start = starts.peek().map(|w| w.travel_mm[0]);
        let Some(&Reverse((speed, end))) = active.peek() else {
            let Some(next) = next_start else { break };
            at = next;
            continue;
        };
        // 已由几何行程的可表示范围约束，末格的后一毫米可表示。
        let next = next_start.map_or(end + 1, |s| s.min(end + 1));
        if let Some(last) = out.last_mut()
            && last.speed_mm_s == speed
            && last.travel_mm[1] + 1 == at
        {
            last.travel_mm[1] = next - 1;
        } else {
            out.try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            out.push(ClearanceSpeedWindow {
                travel_mm: [at, next - 1],
                speed_mm_s: speed,
            });
        }
        at = next;
    }
    Ok(out)
}

// 仅对已经通过转向求证的有限正速度调用。乘法接近整数边界时用一次融合运算
// 检查差值，防止先舍入到浮点整数带来多一毫米每秒的准入。
fn admitted_speed_mm_s(speed_m_s: f64) -> u32 {
    let speed = ((speed_m_s * 1_000.) as u32).min(MAX_SPEED_MM_S);
    if speed_m_s.mul_add(1_000., -f64::from(speed)) < 0. {
        speed - 1
    } else {
        speed
    }
}

impl TrafficWorld {
    /// 按世界显式提供的曲线、原候选车型和本世界固定步长准备只读限速计划。
    /// 完整准入后再分别按 `trial.limits.sweep` 和 `trial.limits.tracks` 求侧向净空与
    /// 累计弧长。生成计划的一次性工作与输出段数、实际几何求证量有关。
    ///
    /// # Errors
    ///
    /// 路线出现或车型不符、轨迹未准入、净空或行程映射不完整、预算或预留不足时返回错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff_speed_plan(
        &self,
        binding: &BoundRouteHandoff,
        source_index: u32,
        trial: &RouteHandoffTrial,
        config: &HandoffDrivingConfig,
    ) -> Result<RouteHandoffSpeedPlan, RouteHandoffSpeedPlanError> {
        let preview = self.preview_route_handoff_at(binding, source_index, trial)?;
        preview.speed_plan(trial, config, self.config().fixed_delta_time_ms() as u32)
    }
}

impl super::RouteHandoffPreview<'_> {
    pub(crate) fn speed_plan(
        &self,
        trial: &RouteHandoffTrial,
        config: &HandoffDrivingConfig,
        fixed_delta_ms: u32,
    ) -> Result<RouteHandoffSpeedPlan, RouteHandoffSpeedPlanError> {
        let path = self
            .trajectory
            .as_ref()
            .ok_or(RouteHandoffSpeedPlanError::Unlocated(self.report.outcome))?;
        let evidence = path.revision.handoff_side_clearances(
            &path.query,
            path.window,
            path.body,
            u64::from(config.max_span_mm.get()),
            trial.limits.sweep,
        );
        let segments =
            evidence
                .segments
                .map_err(|reason| RouteHandoffSpeedPlanError::Clearance {
                    reason,
                    range_mm: evidence.unproven_range_mm,
                    work: evidence.work,
                })?;
        let (ends, speeds) = speed_runs(
            segments
                .iter()
                .map(|s| (s.range_mm[1], s.clearance.map(|c| c.lower_millimetres()))),
            &config.curve,
        )?;
        let marks = path.revision.handoff_travel_marks(
            &path.query,
            path.window,
            &ends,
            u64::from(config.travel_tolerance_mm.get()),
            trial.limits.tracks,
        );
        let distances = marks
            .distances
            .map_err(|reason| RouteHandoffSpeedPlanError::Travel {
                reason,
                work: marks.work,
            })?;
        let mut start_mm = 0;
        let mut spans = Vec::new();
        for (end, speed) in distances.into_iter().zip(speeds) {
            let [low_mm, high_mm] = end.millimetre_cells();
            if speed < MAX_SPEED_MM_S {
                spans
                    .try_reserve(1)
                    .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
                spans.push(ClearanceSpeedWindow {
                    travel_mm: [start_mm, high_mm],
                    speed_mm_s: speed,
                });
            }
            start_mm = low_mm;
        }
        Ok(RouteHandoffSpeedPlan {
            windows: minimum_envelope(spans)?,
            length_lower_um: start_mm * 1_000,
            admitted_speed_mm_s: admitted_speed_mm_s(trial.speed_m_s),
            fixed_delta_ms,
            comfort_decel: path.comfort_decel,
            admission_work: self.report.work
                + self.side_access_work
                + self.stop_line_work
                + self
                    .report
                    .geometry
                    .map_or(0, |g| g.tracks.work + g.sweep.work),
            clearance_work: evidence.work,
            travel_work: marks.work,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changes_in_quantized_speed_and_the_endpoint_require_travel_marks() {
        use crate::ClearanceSpeedPoint;

        let curve = ClearanceSpeedCurve::try_new(vec![
            ClearanceSpeedPoint {
                clearance_mm: 0,
                speed_mm_s: 1_000,
            },
            ClearanceSpeedPoint {
                clearance_mm: 1,
                speed_mm_s: MAX_SPEED_MM_S,
            },
        ])
        .unwrap();
        let gaps_mm = [None, Some(0), Some(0), None, Some(1000), Some(0), None];
        let segments = gaps_mm
            .into_iter()
            .enumerate()
            .map(|(i, gap)| (i as u64 + 1, gap));
        let (marks, speeds) = speed_runs(segments, &curve).unwrap();
        assert_eq!(marks, [1, 3, 5, 6, 7]);
        assert_eq!(
            speeds,
            [MAX_SPEED_MM_S, 1_000, MAX_SPEED_MM_S, 1_000, MAX_SPEED_MM_S]
        );
    }

    #[test]
    fn admitted_speed_never_rounds_up_across_an_integer_boundary() {
        assert_eq!(2.001_f64 * 1_000., 2_001.);
        assert_eq!(admitted_speed_mm_s(2.001), 2_000);
        assert_eq!(admitted_speed_mm_s(2.001_f64.next_up()), 2_001);
        assert_eq!(admitted_speed_mm_s(0.000_99), 0);
        assert_eq!(admitted_speed_mm_s(120.), MAX_SPEED_MM_S);
    }

    #[test]
    fn overlapping_closed_windows_keep_the_lowest_limit_at_each_millimetre() {
        let spans = [
            (10, 15, 120),
            (0, 10, 400),
            (8, 12, 600),
            (13, 18, 120),
            (20, 25, 100),
        ]
        .map(|(start, end, speed)| ClearanceSpeedWindow {
            travel_mm: [start, end],
            speed_mm_s: speed,
        });
        let actual = minimum_envelope(spans.to_vec()).unwrap();
        for at in 0..=30 {
            let limits_at = |windows: &[ClearanceSpeedWindow]| {
                windows
                    .iter()
                    .filter(|w| (w.travel_mm[0]..=w.travel_mm[1]).contains(&at))
                    .map(|w| w.speed_mm_s)
                    .min()
            };
            assert_eq!(limits_at(&actual), limits_at(&spans), "行程 {at} 毫米");
        }
        assert_eq!(
            actual.iter().map(|w| w.travel_mm).collect::<Vec<_>>(),
            [[0, 9], [10, 18], [20, 25]]
        );
    }
}
