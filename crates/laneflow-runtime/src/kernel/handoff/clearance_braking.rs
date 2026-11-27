//! 沿候选真实后轴行程，对当前与前方的侧向净空限速做一拍服务制动前视。
use laneflow_motion_kernel::{braking_distance_to_speed_um, stop_distance_um};
use laneflow_static_contract::{
    MAX_ACCEL_METERS_PER_SECOND_SQUARED, MAX_SPEED_MM_S, MIN_ACCEL_METERS_PER_SECOND_SQUARED,
};
use thiserror::Error;

use super::clearance_plan::{ClearanceSpeedWindow, RouteHandoffSpeedPlan};

/// 本次求界采用的纵向运动，提交时须遵守对应的速度与行程关系。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteHandoffBrakingMotion {
    /// 按整数首尾车速在整拍匀变速。
    Trapezoid,
    /// 从拍初起按车型的服务减速度减至零，拍内剩余时间静止。拍初已静止时行程为零。
    InTickStop,
}

/// 一种满足所查净空与服务制动约束的单拍运动。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteHandoffBrakingStep {
    /// 用于计算速度与行程上界的运动方式。
    pub motion: RouteHandoffBrakingMotion,
    /// 在输入提案以内通过前视的最大整数终速，单位毫米每秒。
    pub next_speed_mm_s: u32,
    /// 本次试算的后轴行程向上取整到微米，仅供核对本拍实际运动。
    pub travel_upper_um: u64,
}

/// 净空限速没有给出覆盖完整一拍的服务制动运动。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffBrakingError {
    /// 位置区间倒序，或速度、步进参数超出交通运行时支持的数值域。
    #[error("净空制动试算的位置区间、速度、步长或减速度无效")]
    InvalidInput,
    /// 查询位置超出已证明可用的候选行程。
    #[error("后轴位置超出已证候选行程")]
    OutsidePlan,
    /// 拍初车速已经超过可能位置上的净空或几何准入上限。
    #[error("拍初速度超过可能位置上的上限 {limit_mm_s} 毫米/秒")]
    AlreadyOverLimit {
        /// 当前有效上限。
        limit_mm_s: u32,
    },
    /// 原提案或前方某个限速要求的减速超过车型的服务制动能力。
    #[error("当前拍没有满足提案与前方限速的服务制动运动")]
    InsufficientBraking,
    /// 运动可能越过候选终点。须在后继上验证剩余运动后才能提交整拍。
    #[error("本拍运动超出候选已证行程，须核对终点续行")]
    NeedsContinuation {
        /// 已满足当前候选内各项限速的提案，行程上界覆盖整拍。后继限制可能要求进一步收紧。
        candidate_step: RouteHandoffBrakingStep,
        /// 从最靠前的后轴位置到候选已证终点的最小剩余行程（微米）。
        remaining_um: u64,
    },
}

struct StepKinematics {
    current: u32,
    delta_ms: u32,
    decel: f32,
    max_drop: u32,
}

impl StepKinematics {
    fn new(current: u32, delta_ms: u32, decel: f32) -> Result<Self, RouteHandoffBrakingError> {
        if current > MAX_SPEED_MM_S
            || !(4..=1_000).contains(&delta_ms)
            || !(MIN_ACCEL_METERS_PER_SECOND_SQUARED..=MAX_ACCEL_METERS_PER_SECOND_SQUARED)
                .contains(&decel)
        {
            return Err(RouteHandoffBrakingError::InvalidInput);
        }
        // binary32 有效数最多 24 位，整数毫秒至多 10 位，乘积在 binary64 中精确。
        let max_drop = (f64::from(decel) * f64::from(delta_ms)) as u32;
        if max_drop == 0 {
            return Err(RouteHandoffBrakingError::InvalidInput);
        }
        Ok(Self {
            current,
            delta_ms,
            decel,
            max_drop,
        })
    }

    // 拍内停稳按服务减速度产生实际位移；其余情况沿整数速度梯形积分。
    fn travel_upper_um(&self, next: u32) -> u64 {
        if next == 0 {
            stop_distance_um(self.current, self.decel)
        } else {
            (u64::from(self.current + next) * u64::from(self.delta_ms)).div_ceil(2)
        }
    }

    // 后续每拍速度均须是整数。k=⌊b·dt⌋ 是可重复提交的最大减幅。先在 O(1)
    // 内求所有完整减速拍的行程上界，再算末拍达到目标速度的位置。k·dt 为奇数时，
    // 每个完整拍的梯形行程均有半微米，逐拍向上取整的进位也必须保留。
    fn braking_room_um(&self, from: u32, limit: u32) -> u64 {
        if from <= limit {
            return 0;
        }
        let k = self.max_drop;
        let full_ticks = (from - limit - 1) / k;
        let last = from - full_ticks * k;
        let twice_mean =
            u64::from(from + last) * u64::from(self.delta_ms) + u64::from((k * self.delta_ms) % 2);
        let full_um = u64::from(full_ticks) * twice_mean / 2;
        let terminal_um = if last <= k {
            braking_distance_to_speed_um(last, limit, self.decel)
        } else {
            let squared_drop = u64::from(last).pow(2) - u64::from(limit).pow(2);
            (squared_drop * u64::from(self.delta_ms)).div_ceil(2 * u64::from(k))
        };
        full_um + terminal_um
    }

    fn room_um(&self, next: u32, limit: u32) -> u64 {
        self.travel_upper_um(next) + self.braking_room_um(next, limit)
    }

    fn can_obey(&self, next: u32, ahead_um: u64, limit: u32) -> bool {
        if self.current.max(next) <= limit {
            return true;
        }
        if next > limit {
            return self.room_um(next, limit) <= ahead_um;
        }
        // 已知 v > l ≥ u。允许在本拍进入限速段，但到达入口时必须已降到 l。
        let needed = if next == 0 {
            braking_distance_to_speed_um(self.current, limit, self.decel)
        } else {
            let speed_squared_drop = u64::from(self.current).pow(2) - u64::from(limit).pow(2);
            (speed_squared_drop * u64::from(self.delta_ms))
                .div_ceil(2 * u64::from(self.current - next))
        };
        needed <= ahead_um
    }

    fn preview(
        &self,
        windows: &[ClearanceSpeedWindow],
        length_lower_um: u64,
        admitted: u32,
        rear_um: [u64; 2],
        proposed: u32,
    ) -> Result<RouteHandoffBrakingStep, RouteHandoffBrakingError> {
        let [rear_lo, rear_hi] = rear_um;
        if rear_lo > rear_hi || proposed > MAX_SPEED_MM_S {
            return Err(RouteHandoffBrakingError::InvalidInput);
        }
        let remaining = length_lower_um
            .checked_sub(rear_hi)
            .ok_or(RouteHandoffBrakingError::OutsidePlan)?;
        let first = windows.partition_point(|w| w.travel_mm[1] < rear_lo / 1_000);
        let mut upcoming = &windows[first..];
        let mut limit = admitted;
        while let Some((active, tail)) = upcoming.split_first()
            && active.travel_mm[0] <= rear_hi / 1_000
        {
            limit = limit.min(active.speed_mm_s);
            upcoming = tail;
        }
        if self.current > limit {
            return Err(RouteHandoffBrakingError::AlreadyOverLimit { limit_mm_s: limit });
        }
        let mut upper = proposed.min(limit);
        let floor = self.current.saturating_sub(self.max_drop);
        if upper < floor {
            return Err(RouteHandoffBrakingError::InsufficientBraking);
        }
        // 到此距离还可停稳，后续限速不会收紧提案。每次收紧后只会缩短前视。
        let mut horizon = if upcoming.is_empty() {
            0
        } else {
            self.room_um(upper, 0)
        };
        for window in upcoming {
            let distance = window.travel_mm[0] * 1_000 - rear_hi;
            if distance > horizon {
                break;
            }
            if self.can_obey(upper, distance, window.speed_mm_s) {
                continue;
            }
            if !self.can_obey(floor, distance, window.speed_mm_s) {
                return Err(RouteHandoffBrakingError::InsufficientBraking);
            }
            // 给定拍初速度，在服务减速可达域内，终速越大，达标所需距离单调非减。
            let mut low = floor;
            while low < upper {
                let mid = low + (upper - low).div_ceil(2);
                if self.can_obey(mid, distance, window.speed_mm_s) {
                    low = mid;
                } else {
                    upper = mid - 1;
                }
            }
            horizon = self.room_um(upper, 0);
        }
        let step = RouteHandoffBrakingStep {
            motion: if upper == 0 {
                RouteHandoffBrakingMotion::InTickStop
            } else {
                RouteHandoffBrakingMotion::Trapezoid
            },
            next_speed_mm_s: upper,
            travel_upper_um: self.travel_upper_um(upper),
        };
        if step.travel_upper_um > remaining {
            return Err(RouteHandoffBrakingError::NeedsContinuation {
                candidate_step: step,
                remaining_um: remaining,
            });
        }
        Ok(step)
    }
}

impl RouteHandoffSpeedPlan {
    /// 本拍按提案行驶，再逐拍采用车型可提交的最大服务减幅，最后一拍内停稳。
    pub(super) fn stop_horizon_um(&self, current: u32, proposed: u32) -> Option<u64> {
        let motion = StepKinematics::new(current, self.fixed_delta_ms, self.comfort_decel).ok()?;
        Some(motion.room_um(proposed.min(self.admitted_speed_mm_s), 0))
    }
    /// 已在同一轨迹上定位到此行程时，可收紧整毫米总长下界。跨越终点仍需另证。
    pub(super) fn preview_from_proven_position(
        &self,
        rear_um: u64,
        current_mm_s: u32,
        proposed_mm_s: u32,
    ) -> Result<RouteHandoffBrakingStep, RouteHandoffBrakingError> {
        StepKinematics::new(current_mm_s, self.fixed_delta_ms, self.comfort_decel)?.preview(
            &self.windows,
            self.length_lower_um.max(rear_um),
            self.admitted_speed_mm_s,
            [rear_um; 2],
            proposed_mm_s,
        )
    }

    /// 后轴累计行程已精确落在整数微米时，求一拍服务制动运动。终速提案的驱动
    /// 加速度须已经纵向控制核对，提案降幅超出服务制动域时返回
    /// [`RouteHandoffBrakingError::InsufficientBraking`]。带定位或量化误差的查询使用
    /// [`Self::preview_braking_step_in`]。
    ///
    /// # Errors
    ///
    /// 拍初已超速、提案或前方降幅无法以服务制动达成、数值无效、已证行程不足时返回原因。
    pub fn preview_braking_step(
        &self,
        rear_travel_um: u64,
        current_mm_s: u32,
        proposed_mm_s: u32,
    ) -> Result<RouteHandoffBrakingStep, RouteHandoffBrakingError> {
        self.preview_braking_step_in([rear_travel_um; 2], current_mm_s, proposed_mm_s)
    }

    /// 按包含真实后轴累计行程的闭区间（微米）求一拍服务制动运动。行程下界越过
    /// 限制末端才解除限速，上界决定到前方限速或终点的最小剩余距离。终速提案
    /// （毫米/秒）的驱动加速度须已核对，减速提案须在服务制动域内。来车、信号等其余约束应另行裁决。
    /// 非零终速按整数首尾车速匀变速，零终速按服务减速度在拍内停稳。再次查询应
    /// 使用实际累计行程或包住它的上下界，不能将各拍位移上界累加作为精确位置。
    ///
    /// # Errors
    ///
    /// 区间倒序、速度无效、已证行程不足、拍初可能超速或无法执行服务制动时返回原因。
    pub fn preview_braking_step_in(
        &self,
        rear_travel_um: [u64; 2],
        current_mm_s: u32,
        proposed_mm_s: u32,
    ) -> Result<RouteHandoffBrakingStep, RouteHandoffBrakingError> {
        StepKinematics::new(current_mm_s, self.fixed_delta_ms, self.comfort_decel)?.preview(
            &self.windows,
            self.length_lower_um,
            self.admitted_speed_mm_s,
            rear_travel_um,
            proposed_mm_s,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(start: u64, end: u64, speed_mm_s: u32) -> ClearanceSpeedWindow {
        ClearanceSpeedWindow {
            travel_mm: [start, end],
            speed_mm_s,
        }
    }

    #[test]
    fn farther_narrow_section_binds_at_exact_service_braking_distance() {
        let motion = StepKinematics::new(8_000, 100, 2.0).unwrap();
        let preview = |start| {
            motion.preview(
                &[window(10_000, 12_000, 7_900), window(start, 19_000, 1_000)],
                30_000_000,
                MAX_SPEED_MM_S,
                [0; 2],
                8_000,
            )
        };
        // (8² − 1²)/(2·2) = 15.75 m。以 2 m/s² 制动一拍，终速为 7.8 m/s。
        assert_eq!(
            preview(15_750),
            Ok(RouteHandoffBrakingStep {
                motion: RouteHandoffBrakingMotion::Trapezoid,
                next_speed_mm_s: 7_800,
                travel_upper_um: 790_000,
            })
        );
        assert_eq!(
            preview(15_749),
            Err(RouteHandoffBrakingError::InsufficientBraking)
        );
        let farther = preview(16_000).unwrap();
        assert!(farther.next_speed_mm_s > 7_800 && farther.next_speed_mm_s < 8_000);
        assert!(motion.can_obey(farther.next_speed_mm_s, 16_000_000, 1_000));
        assert!(!motion.can_obey(farther.next_speed_mm_s + 1, 16_000_000, 1_000));
    }

    #[test]
    fn clearance_limit_is_held_until_the_rear_leaves_the_closed_window() {
        let windows = [window(1_000, 2_000, 1_000)];
        let preview = |rear, speed, proposal| {
            StepKinematics::new(speed, 100, 2.0)
                .unwrap()
                .preview(&windows, 6_000_000, 1_500, [rear; 2], proposal)
        };
        assert_eq!(
            preview(2_000_000, 1_000, 1_501).unwrap().next_speed_mm_s,
            1_000
        );
        assert_eq!(
            preview(2_000_999, 1_000, 1_501).unwrap().next_speed_mm_s,
            1_000
        );
        assert_eq!(
            preview(2_001_000, 1_000, 1_501).unwrap().next_speed_mm_s,
            1_500
        );
        assert_eq!(
            preview(2_000_000, 1_001, 1_000),
            Err(RouteHandoffBrakingError::AlreadyOverLimit { limit_mm_s: 1_000 })
        );
        assert_eq!(
            preview(5_000_000, 1_501, 1_500),
            Err(RouteHandoffBrakingError::AlreadyOverLimit { limit_mm_s: 1_500 })
        );
        assert_eq!(
            preview(5_000_000, 1_500, 1_299),
            Err(RouteHandoffBrakingError::InsufficientBraking)
        );
    }

    #[test]
    fn crossing_inside_a_tick_requires_the_actual_entry_speed_to_fit() {
        let motion = StepKinematics::new(1_200, 100, 4.0).unwrap();
        let preview = |start| {
            motion.preview(
                &[window(start, 200, 1_000)],
                1_000_000,
                MAX_SPEED_MM_S,
                [0; 2],
                1_200,
            )
        };
        // 拍中 50 ms 达到 1 m/s，此时累计 55 mm；拍末驶过 100 mm。
        assert_eq!(
            preview(55),
            Ok(RouteHandoffBrakingStep {
                motion: RouteHandoffBrakingMotion::Trapezoid,
                next_speed_mm_s: 800,
                travel_upper_um: 100_000,
            })
        );
        assert_eq!(
            preview(54),
            Err(RouteHandoffBrakingError::InsufficientBraking)
        );
    }

    #[test]
    fn future_stop_crosses_a_positive_limit_at_the_squared_speed_difference() {
        let motion = StepKinematics::new(8, 4, 1.0).unwrap();
        let preview = |room: u64| {
            motion.preview(
                &[window(1, 2, 3)],
                3_000,
                MAX_SPEED_MM_S,
                [1_000 - room; 2],
                8,
            )
        };
        // 首拍由 8 降至 4 mm/s，行程 24 µm。下一拍能以服务制动停稳，
        // 在该拍 3.5 µm 处已经降至 3 mm/s，距两拍起点共需向上取整到 28 µm。
        assert_eq!(
            preview(28),
            Ok(RouteHandoffBrakingStep {
                motion: RouteHandoffBrakingMotion::Trapezoid,
                next_speed_mm_s: 4,
                travel_upper_um: 24,
            })
        );
        assert_eq!(
            preview(27),
            Err(RouteHandoffBrakingError::InsufficientBraking)
        );
    }

    #[test]
    fn integer_speed_drops_stay_feasible_when_rechecked_from_every_committed_tick() {
        // 0.7 m/s²、5 ms 的理想减幅约 3.5 mm/s；实际整数速度最多下降 3 mm/s。
        // 四拍梯形行程均落在半微米，逐拍向外量化的位置区间必须仍可复核。
        let windows = [window(1, 2, 8)];
        let mut rear = [718; 2];
        let mut speed = 20;
        for (next, travel) in [(17, 93), (14, 78), (11, 63), (8, 48)] {
            let motion = StepKinematics::new(speed, 5, 0.7).unwrap();
            let result = motion.preview(&windows, 5_000, 100, rear, speed).unwrap();
            assert_eq!(
                result,
                RouteHandoffBrakingStep {
                    motion: RouteHandoffBrakingMotion::Trapezoid,
                    next_speed_mm_s: next,
                    travel_upper_um: travel,
                }
            );
            rear[0] += u64::from(speed + next) * 5 / 2;
            rear[1] += travel;
            speed = next;
        }
        assert_eq!(rear, [996, windows[0].travel_mm[0] * 1_000]);
        // 上界进入下一毫米格时，下界仍在末端格，继续限速。
        let last = StepKinematics::new(speed, 5, 0.7).unwrap();
        let exit = [window(0, 0, 8)];
        assert_eq!(
            last.preview(&exit, 5_000, 100, rear, 10)
                .unwrap()
                .next_speed_mm_s,
            8
        );
        let initial = StepKinematics::new(20, 5, 0.7).unwrap();
        assert_eq!(
            initial.preview(&windows, 5_000, 100, [718, 719], 20),
            Err(RouteHandoffBrakingError::InsufficientBraking)
        );
    }

    #[test]
    fn every_window_within_the_position_interval_is_active() {
        let windows = [window(2, 7, 500), window(9, 19, 200), window(21, 30, 400)];
        let preview = |range, speed| {
            StepKinematics::new(speed, 100, 2.0)
                .unwrap()
                .preview(&windows, 1_000_000, 500, range, 300)
        };
        // 两端均处在窗口外，与中间窗口相交仍须取三个上限中的最小值。
        assert_eq!(
            preview([1_000, 31_000], 201),
            Err(RouteHandoffBrakingError::AlreadyOverLimit { limit_mm_s: 200 })
        );
        assert_eq!(preview([19_000, 31_000], 200).unwrap().next_speed_mm_s, 200);
        assert_eq!(preview([20_000, 31_000], 200).unwrap().next_speed_mm_s, 300);
        assert_eq!(
            preview([31, 30], 200),
            Err(RouteHandoffBrakingError::InvalidInput)
        );
        assert_eq!(
            preview([30, 1_000_001], 200),
            Err(RouteHandoffBrakingError::OutsidePlan)
        );
        assert_eq!(
            preview([19_000, 999_999], 200),
            Err(RouteHandoffBrakingError::NeedsContinuation {
                candidate_step: RouteHandoffBrakingStep {
                    motion: RouteHandoffBrakingMotion::Trapezoid,
                    next_speed_mm_s: 200,
                    travel_upper_um: 20_000,
                },
                remaining_um: 1,
            })
        );
    }

    #[test]
    fn stop_within_tick_and_candidate_end_have_distinct_boundaries() {
        let motion = StepKinematics::new(100, 100, 2.0).unwrap();
        let preview = |rear, length| motion.preview(&[], length, MAX_SPEED_MM_S, [rear; 2], 0);
        // 50 ms 停稳。服务制动行程 2.5 mm，不能从量化终速零恢复成整拍梯形 5 mm。
        let stop = RouteHandoffBrakingStep {
            motion: RouteHandoffBrakingMotion::InTickStop,
            next_speed_mm_s: 0,
            travel_upper_um: 2_500,
        };
        assert_eq!(preview(0, 2_500), Ok(stop));
        assert_eq!(
            preview(0, 2_499),
            Err(RouteHandoffBrakingError::NeedsContinuation {
                candidate_step: stop,
                remaining_um: 2_499,
            })
        );
        assert_eq!(preview(2, 1), Err(RouteHandoffBrakingError::OutsidePlan));
        let upper = StepKinematics::new(MAX_SPEED_MM_S, 1_000, 0.5).unwrap();
        assert_eq!(
            upper
                .preview(&[], u64::MAX, MAX_SPEED_MM_S, [0; 2], MAX_SPEED_MM_S)
                .unwrap()
                .travel_upper_um,
            100_000_000
        );
    }
}
