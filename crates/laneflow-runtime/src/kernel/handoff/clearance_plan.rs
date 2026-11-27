use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::num::NonZeroU64;

use laneflow_static_contract::MAX_SPEED_MM_S;
use laneflow_static_network::vehicle::{Failure, HandoffOutcome, SideClearanceSegment};
use thiserror::Error;

use super::{BoundRouteHandoff, ClearanceSpeedCurve, RouteHandoffTrial, RouteHandoffTrialError};
use crate::TrafficWorld;

/// 动态换道的显式驾驶配置。拍间试算从调用方读取；两个尺度都参与行为重建。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffDrivingConfig {
    /// 从已证绝对侧向余量换算速度上限。
    pub curve: ClearanceSpeedCurve,
    /// 每个侧向几何段的最大后轴参数跨度（纳米）。
    pub max_span_nm: NonZeroU64,
    /// 同一候选累计三维行程求界的最大宽度（纳米）。
    pub travel_tolerance_nm: NonZeroU64,
}

/// 后轴实际累计行程中须遵守同一侧向限速的闭区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearanceSpeedWindow {
    /// 进入向下、离开向上取整到微米；两端都受此上限约束。
    pub travel_um: [u64; 2],
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

    /// 可确认尚在候选上的最大后轴累计行程，单位微米。
    #[must_use]
    pub fn length_lower_um(&self) -> u64 {
        self.length_lower_um
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
    #[error("侧向净空未证：{reason:?}，候选区间 {range_nm:?}，工作 {work}")]
    Clearance {
        /// 几何失败原因。
        reason: Failure,
        /// 首个未证的有向候选参数区间（纳米）。
        range_nm: Option<[u64; 2]>,
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
    segments: &[SideClearanceSegment<'_>],
    curve: &ClearanceSpeedCurve,
) -> Result<(Vec<u64>, Vec<u32>), RouteHandoffSpeedPlanError> {
    let mut ends = Vec::new();
    let mut speeds = Vec::new();
    for segment in segments {
        let speed = segment.clearance.map_or(MAX_SPEED_MM_S, |c| {
            curve.speed_cap_mm_s(c.lower_micrometres())
        });
        if speeds.last() == Some(&speed)
            && let Some(last) = ends.last_mut()
        {
            *last = segment.range_nm[1];
        } else {
            ends.try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            speeds
                .try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            ends.push(segment.range_nm[1]);
            speeds.push(speed);
        }
    }
    Ok((ends, speeds))
}

// 边界求界可互相重叠。按闭区间取全部有效上限的最小值，解除时刻是末端后一微米。
fn minimum_envelope(
    mut spans: Vec<ClearanceSpeedWindow>,
) -> Result<Vec<ClearanceSpeedWindow>, RouteHandoffSpeedPlanError> {
    spans.sort_unstable_by_key(|w| w.travel_um[0]);
    let Some(first) = spans.first() else {
        return Ok(spans);
    };
    let mut at = first.travel_um[0];
    let mut starts = spans.iter().peekable();
    let mut active = BinaryHeap::new();
    active
        .try_reserve(spans.len())
        .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
    let mut out: Vec<ClearanceSpeedWindow> = Vec::new();
    loop {
        while let Some(w) = starts.next_if(|w| w.travel_um[0] <= at) {
            active.push(Reverse((w.speed_mm_s, w.travel_um[1])));
        }
        while active.peek().is_some_and(|&Reverse((_, end))| end < at) {
            active.pop();
        }
        let next_start = starts.peek().map(|w| w.travel_um[0]);
        let Some(&Reverse((speed, end))) = active.peek() else {
            let Some(next) = next_start else { break };
            at = next;
            continue;
        };
        // 来自 u64 纳米上界，向上量化到微米后至少保留三个十进制数量级的余量。
        let next = next_start.map_or(end + 1, |s| s.min(end + 1));
        if let Some(last) = out.last_mut()
            && last.speed_mm_s == speed
            && last.travel_um[1] + 1 == at
        {
            last.travel_um[1] = next - 1;
        } else {
            out.try_reserve(1)
                .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
            out.push(ClearanceSpeedWindow {
                travel_um: [at, next - 1],
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
        let path = preview
            .trajectory
            .as_ref()
            .ok_or(RouteHandoffSpeedPlanError::Unlocated(
                preview.report.outcome,
            ))?;
        let evidence = path.revision.handoff_side_clearances(
            &path.query,
            path.window,
            path.body,
            config.max_span_nm.get(),
            trial.limits.sweep,
        );
        let segments =
            evidence
                .segments
                .map_err(|reason| RouteHandoffSpeedPlanError::Clearance {
                    reason,
                    range_nm: evidence.unproven_range_nm,
                    work: evidence.work,
                })?;
        let (ends, speeds) = speed_runs(&segments, &config.curve)?;
        let marks = path.revision.handoff_travel_marks(
            &path.query,
            path.window,
            &ends,
            config.travel_tolerance_nm.get(),
            trial.limits.tracks,
        );
        let distances =
            marks
                .distances_nm
                .map_err(|reason| RouteHandoffSpeedPlanError::Travel {
                    reason,
                    work: marks.work,
                })?;
        let mut start_nm = 0;
        let mut spans = Vec::new();
        for (end_nm, speed) in distances.into_iter().zip(speeds) {
            if speed < MAX_SPEED_MM_S {
                spans
                    .try_reserve(1)
                    .map_err(|_| RouteHandoffSpeedPlanError::Allocation)?;
                spans.push(ClearanceSpeedWindow {
                    travel_um: [start_nm / 1_000, end_nm[1].div_ceil(1_000)],
                    speed_mm_s: speed,
                });
            }
            start_nm = end_nm[0];
        }
        let profile = path
            .revision
            .identity()
            .ordinal(trial.profile)
            .and_then(|p| self.traffic().relations().vehicle_profile(p))
            .ok_or(RouteHandoffTrialError::UnknownProfile)?;
        Ok(RouteHandoffSpeedPlan {
            windows: minimum_envelope(spans)?,
            length_lower_um: start_nm / 1_000,
            admitted_speed_mm_s: admitted_speed_mm_s(trial.speed_m_s),
            fixed_delta_ms: self.config().fixed_delta_time_ms() as u32,
            comfort_decel: profile.comfort_decel(),
            admission_work: preview.report.work
                + preview.side_access_work
                + preview
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
    fn admitted_speed_never_rounds_up_across_an_integer_boundary() {
        assert_eq!(2.001_f64 * 1_000., 2_001.);
        assert_eq!(admitted_speed_mm_s(2.001), 2_000);
        assert_eq!(admitted_speed_mm_s(2.001_f64.next_up()), 2_001);
        assert_eq!(admitted_speed_mm_s(0.000_99), 0);
        assert_eq!(admitted_speed_mm_s(120.), MAX_SPEED_MM_S);
    }

    #[test]
    fn overlapping_closed_windows_keep_the_lowest_limit_at_each_micrometre() {
        let spans = [
            (10, 15, 120),
            (0, 10, 400),
            (8, 12, 600),
            (13, 18, 120),
            (20, 25, 100),
        ]
        .map(|(start, end, speed)| ClearanceSpeedWindow {
            travel_um: [start, end],
            speed_mm_s: speed,
        });
        let actual = minimum_envelope(spans.to_vec()).unwrap();
        for at in 0..=30 {
            let limits_at = |windows: &[ClearanceSpeedWindow]| {
                windows
                    .iter()
                    .filter(|w| (w.travel_um[0]..=w.travel_um[1]).contains(&at))
                    .map(|w| w.speed_mm_s)
                    .min()
            };
            assert_eq!(limits_at(&actual), limits_at(&spans), "行程 {at} 微米");
        }
        assert_eq!(
            actual.iter().map(|w| w.travel_um).collect::<Vec<_>>(),
            [[0, 9], [10, 18], [20, 25]]
        );
    }
}
