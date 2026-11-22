//! 超 profile 减速度的具名例外归因（#1487）。
//!
//! 整数几何 §5「正常停车定义与例外」要求超出 `b_e·Δt + ε_v` 的样本逐项归因。
//! 判据只读观测侧的拍初/拍末公开状态与宿主按冻结计划维护的期望停车绑定、
//! 路终位置，不调用 Runtime 的停止求解器、例外判定或 arrival 自报。不满足
//! 任何判据的样本记为未归因；未归因非零时运行失败，不隐藏超界。

use laneflow_runtime::{VehicleState, VehicleStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Error;

/// 路线上的一个整数位置：route occurrence 下标与该边进度（毫米）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoutePoint {
    pub edge_index: u32,
    pub progress_mm: u32,
}

/// 单个样本可用的独立判据输入：路线终点，以及该车持有停车预约时的停车入口位置。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Exits {
    pub route_end: Option<RoutePoint>,
    pub parking_entrance: Option<RoutePoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exception {
    /// 停车到位拍清零：持有停车预约，拍末 Active、静止于预约的停车入口。
    ParkingArrivalZero,
    /// 路终 Completed 拍：拍初 Active，拍末 Completed、静止于路线终点。
    RouteEndCompleted,
    Unattributed,
}

/// 判据读取的拍末公开状态。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Observed {
    pub status: VehicleStatus,
    pub edge_index: u32,
    pub progress_mm: u32,
    pub carry_um: u16,
    pub speed_mm_s: u32,
}

impl From<VehicleState> for Observed {
    fn from(state: VehicleState) -> Self {
        Self {
            status: state.status(),
            edge_index: state.route_edge_index(),
            progress_mm: state.progress_mm(),
            carry_um: state.carry_um(),
            speed_mm_s: state.speed_mm_s(),
        }
    }
}

pub(crate) fn classify(before: VehicleStatus, after: Observed, exits: Exits) -> Exception {
    let at = |point: RoutePoint| {
        after.edge_index == point.edge_index
            && after.progress_mm == point.progress_mm
            && after.carry_um == 0
            && after.speed_mm_s == 0
    };
    let active_before = before == VehicleStatus::Active;
    match after.status {
        VehicleStatus::Active if active_before && exits.parking_entrance.is_some_and(at) => {
            Exception::ParkingArrivalZero
        }
        VehicleStatus::Completed if active_before && exits.route_end.is_some_and(at) => {
            Exception::RouteEndCompleted
        }
        _ => Exception::Unattributed,
    }
}

/// 一个类别的样本数与最大减速度（mm/s²，向上取整）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tally {
    pub count: u64,
    pub max_mm_s2: u64,
}

impl Tally {
    fn record(&mut self, decel_mm_s2: u64) {
        self.count += 1;
        self.max_mm_s2 = self.max_mm_s2.max(decel_mm_s2);
    }

    fn json(self) -> Value {
        json!({"count":self.count,"max_observed_deceleration_mm_s2":self.max_mm_s2})
    }
}

/// 逐类别归因计数；各类别之和恒等于 `deceleration_above_profile_count`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attribution {
    pub parking_arrival_zero: Tally,
    pub route_end_completed: Tally,
    pub unattributed: Tally,
}

impl Attribution {
    pub(crate) fn record(&mut self, exception: Exception, decel_mm_s2: u64) {
        match exception {
            Exception::ParkingArrivalZero => &mut self.parking_arrival_zero,
            Exception::RouteEndCompleted => &mut self.route_end_completed,
            Exception::Unattributed => &mut self.unattributed,
        }
        .record(decel_mm_s2);
    }

    pub(crate) fn total(&self) -> u64 {
        self.parking_arrival_zero.count + self.route_end_completed.count + self.unattributed.count
    }

    pub(crate) fn json(&self) -> Value {
        json!({
            "parking_arrival_zero":self.parking_arrival_zero.json(),
            "route_end_completed":self.route_end_completed.json(),
            "unattributed":self.unattributed.json(),
        })
    }

    /// 回执前置条件：类别之和等于总计数，且没有未归因样本。
    pub(crate) fn require_attributed(&self, above_profile: u64) -> Result<(), Error> {
        if self.total() != above_profile {
            return Err(Error::Invalid(format!(
                "超 profile 减速归因之和 {} 与总计数 {above_profile} 不符",
                self.total()
            )));
        }
        if self.unattributed.count > 0 {
            return Err(Error::Invalid(format!(
                "超 profile 减速存在未归因样本：{}（最大 {} mm/s²），归因：{}",
                self.unattributed.count,
                self.unattributed.max_mm_s2,
                self.json()
            )));
        }
        Ok(())
    }
}

/// 单次运行的减速诊断：最大观测减速度、超 profile 样本计数与逐类别归因。
/// 检查点宿主材料携带本结构，恢复续跑不重复也不丢失计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Counters {
    pub max_mm_s2: u64,
    pub above_profile: u64,
    pub attribution: Attribution,
}

impl Counters {
    /// 单车单拍：累计最大观测减速度（mm/s²，向上取整）；降幅超出 `limit_mm_s`
    /// （安装 profile 紧急减速上限 × 步长 + 1）的样本计数并归因。
    pub(crate) fn record(
        &mut self,
        before: VehicleState,
        after: VehicleState,
        dt: u32,
        limit_mm_s: u64,
        exits: Exits,
    ) {
        let drop = u64::from(before.speed_mm_s().saturating_sub(after.speed_mm_s()));
        let decel_mm_s2 = (drop * 1_000).div_ceil(u64::from(dt));
        self.max_mm_s2 = self.max_mm_s2.max(decel_mm_s2);
        if drop > limit_mm_s {
            self.above_profile += 1;
            self.attribution
                .record(classify(before.status(), after.into(), exits), decel_mm_s2);
        }
    }

    /// 回执字段：总计数与最大值之外逐类别对齐例外清单；未归因非零时拒绝出回执。
    pub(crate) fn receipt(&self) -> Result<Value, Error> {
        self.attribution.require_attributed(self.above_profile)?;
        Ok(json!({
            "max_observed_deceleration_mm_s2":self.max_mm_s2,
            "deceleration_above_profile_count":self.above_profile,
            "deceleration_above_profile_attribution":self.attribution.json(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const END: RoutePoint = RoutePoint {
        edge_index: 1,
        progress_mm: 5_130_000,
    };
    const ENTRANCE: RoutePoint = RoutePoint {
        edge_index: 1,
        progress_mm: 5_000,
    };

    #[test]
    fn attribution_requires_every_sample_named_and_sums_to_total() {
        let mut attribution = Attribution::default();
        attribution.record(Exception::ParkingArrivalZero, 13_563);
        attribution.record(Exception::ParkingArrivalZero, 9_000);
        attribution.record(Exception::RouteEndCompleted, 7_000);
        assert_eq!(attribution.total(), 3);
        assert_eq!(attribution.parking_arrival_zero.max_mm_s2, 13_563);
        attribution.require_attributed(3).unwrap();
        assert!(attribution.require_attributed(4).is_err());
        attribution.record(Exception::Unattributed, 400_313);
        assert!(attribution.require_attributed(4).is_err());
        assert_eq!(
            attribution.json()["unattributed"]["max_observed_deceleration_mm_s2"],
            400_313
        );
    }

    fn observed(status: VehicleStatus, point: RoutePoint, speed_mm_s: u32) -> Observed {
        Observed {
            status,
            edge_index: point.edge_index,
            progress_mm: point.progress_mm,
            carry_um: 0,
            speed_mm_s,
        }
    }

    #[test]
    fn classify_names_only_stationary_arrivals_at_the_declared_points() {
        use VehicleStatus::{Active, Completed};
        let both = Exits {
            route_end: Some(END),
            parking_entrance: Some(ENTRANCE),
        };
        assert_eq!(
            classify(Active, observed(Active, ENTRANCE, 0), both),
            Exception::ParkingArrivalZero
        );
        assert_eq!(
            classify(Active, observed(Completed, END, 0), both),
            Exception::RouteEndCompleted
        );
        // 无预约、未静止、带微米余数、位置不符或拍初非 Active 均不归入例外。
        let no_parking = Exits {
            parking_entrance: None,
            ..both
        };
        assert_eq!(
            classify(Active, observed(Active, ENTRANCE, 0), no_parking),
            Exception::Unattributed
        );
        assert_eq!(
            classify(Active, observed(Active, ENTRANCE, 1), both),
            Exception::Unattributed
        );
        let mut carried = observed(Active, ENTRANCE, 0);
        carried.carry_um = 1;
        assert_eq!(classify(Active, carried, both), Exception::Unattributed);
        assert_eq!(
            classify(Active, observed(Active, END, 0), both),
            Exception::Unattributed
        );
        assert_eq!(
            classify(Completed, observed(Completed, END, 0), both),
            Exception::Unattributed
        );
        assert_eq!(
            classify(Active, observed(Active, ENTRANCE, 0), Exits::default()),
            Exception::Unattributed
        );
    }
}
