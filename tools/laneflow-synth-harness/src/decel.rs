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

/// 判超所需的运动边界：安装 profile 的紧急减速度与固定步长。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Motion {
    pub emergency_mm_s2: u64,
    pub dt_ms: u32,
}

impl Motion {
    /// 判超阈值 `⌈b_e·Δt⌉ + 1` mm/s（`ε_v = 1 mm/s`，#1243 T5）。`b_e` 取安装
    /// profile：W 族 6 m/s²，L 族 9 m/s²。
    pub(crate) fn drop_limit_mm_s(self) -> u64 {
        (self.emergency_mm_s2 * u64::from(self.dt_ms)).div_ceil(1_000) + 1
    }

    /// 终态清零只吸收至多一拍的整数落地越程：拍初速度扣除 `ε_v` 后，紧急制动
    /// 距离不超过拍初到该位置的剩余距离加一拍行程，即
    /// `(v−ε_v)² ≤ 2·b_e·(d + v·Δt)`。拍初须与该位置同一 route occurrence 且不越过
    /// 它（整数几何 §5：carry 并入后的整数落地可以先于 SI 预判耗尽一拍）。
    fn overrun_within_one_tick(self, before: Observed, point: RoutePoint) -> bool {
        let before_um = u64::from(before.progress_mm) * 1_000 + u64::from(before.carry_um);
        let point_um = u64::from(point.progress_mm) * 1_000;
        if before.edge_index != point.edge_index || before_um > point_um {
            return false;
        }
        let speed = u128::from(before.speed_mm_s.saturating_sub(1));
        let room_um = u128::from(point_um - before_um)
            + u128::from(before.speed_mm_s) * u128::from(self.dt_ms);
        speed * speed * 1_000 <= 2 * u128::from(self.emergency_mm_s2) * room_um
    }
}

pub(crate) fn classify(
    before: Observed,
    after: Observed,
    exits: Exits,
    motion: Motion,
) -> Exception {
    let at = |point: RoutePoint| {
        after.edge_index == point.edge_index
            && after.progress_mm == point.progress_mm
            && after.carry_um == 0
            && after.speed_mm_s == 0
            && motion.overrun_within_one_tick(before, point)
    };
    let active_before = before.status == VehicleStatus::Active;
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
    /// 单车单拍：累计最大观测减速度（mm/s²，向上取整）；降幅超出判超阈值的样本
    /// 计数并归因。
    pub(crate) fn record(
        &mut self,
        before: VehicleState,
        after: VehicleState,
        motion: Motion,
        exits: Exits,
    ) {
        let drop = u64::from(before.speed_mm_s().saturating_sub(after.speed_mm_s()));
        let decel_mm_s2 = (drop * 1_000).div_ceil(u64::from(motion.dt_ms));
        self.max_mm_s2 = self.max_mm_s2.max(decel_mm_s2);
        if drop > motion.drop_limit_mm_s() {
            self.above_profile += 1;
            self.attribution.record(
                classify(before.into(), after.into(), exits, motion),
                decel_mm_s2,
            );
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

/// 消费侧准入（#1487）：运行回执必须携带逐类别归因，三个类别齐全且为非负整数，
/// 各类别之和等于 `deceleration_above_profile_count`，未归因为零。缺少归因证据的
/// 旧报告即使携带当前语义协议也拒绝。
pub(crate) fn check_receipt(report: &Value) -> Result<(), Error> {
    let invalid = |detail: String| Error::Invalid(format!("回执减速归因不符：{detail}"));
    let total = report["deceleration_above_profile_count"]
        .as_u64()
        .ok_or_else(|| invalid("缺少 deceleration_above_profile_count".into()))?;
    let attribution = report["deceleration_above_profile_attribution"]
        .as_object()
        .ok_or_else(|| invalid("缺少 deceleration_above_profile_attribution".into()))?;
    const CATEGORIES: [&str; 3] = [
        "parking_arrival_zero",
        "route_end_completed",
        "unattributed",
    ];
    if attribution.len() != CATEGORIES.len() {
        return Err(invalid(format!("类别集合不符：{attribution:?}")));
    }
    let mut sum = 0_u64;
    for category in CATEGORIES {
        let tally = &attribution
            .get(category)
            .ok_or_else(|| invalid(format!("缺少类别 {category}")))?;
        let count = tally["count"]
            .as_u64()
            .ok_or_else(|| invalid(format!("{category} 缺少 count")))?;
        tally["max_observed_deceleration_mm_s2"]
            .as_u64()
            .ok_or_else(|| invalid(format!("{category} 缺少最大减速度")))?;
        if tally.as_object().is_none_or(|fields| fields.len() != 2) {
            return Err(invalid(format!("{category} 字段集合不符")));
        }
        if category == "unattributed" && count > 0 {
            return Err(invalid(format!("存在未归因样本 {count}")));
        }
        sum = sum
            .checked_add(count)
            .ok_or_else(|| invalid("计数溢出".into()))?;
    }
    if sum != total {
        return Err(invalid(format!("类别之和 {sum} 与总计数 {total} 不符")));
    }
    Ok(())
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

    #[test]
    fn receipt_check_rejects_missing_mismatched_and_unattributed_evidence() {
        let mut counters = Counters {
            above_profile: 2,
            ..Counters::default()
        };
        counters
            .attribution
            .record(Exception::ParkingArrivalZero, 13_563);
        counters
            .attribution
            .record(Exception::RouteEndCompleted, 7_000);
        let good = counters.receipt().unwrap();
        check_receipt(&good).unwrap();
        // 旧报告：只有总数，没有归因字段（如 4d672eea 的 lf-synth-execution-2 回执）。
        let mut legacy = good.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("deceleration_above_profile_attribution");
        assert!(check_receipt(&legacy).is_err());
        let mut mismatched = good.clone();
        mismatched["deceleration_above_profile_count"] = json!(3);
        assert!(check_receipt(&mismatched).is_err());
        let mut unattributed = good.clone();
        unattributed["deceleration_above_profile_count"] = json!(3);
        unattributed["deceleration_above_profile_attribution"]["unattributed"]["count"] = json!(1);
        assert!(check_receipt(&unattributed).is_err());
        let mut missing = good.clone();
        missing["deceleration_above_profile_attribution"]
            .as_object_mut()
            .unwrap()
            .remove("route_end_completed");
        assert!(check_receipt(&missing).is_err());
        let mut extra = good;
        extra["deceleration_above_profile_attribution"]["leader_projection"] =
            json!({"count":0,"max_observed_deceleration_mm_s2":0});
        assert!(check_receipt(&extra).is_err());
        // 生产侧同一前置条件：未归因非零时不产生回执。
        let mut rejected = Counters {
            above_profile: 1,
            ..Counters::default()
        };
        rejected
            .attribution
            .record(Exception::Unattributed, 400_313);
        assert!(rejected.receipt().is_err());
    }

    const W16: Motion = Motion {
        emergency_mm_s2: 6_000,
        dt_ms: 16,
    };

    /// 拍初：Active，位于 `point` 前 `behind_um`，速度 `speed_mm_s`。
    fn approaching(point: RoutePoint, behind_um: u64, speed_mm_s: u32) -> Observed {
        let at_um = u64::from(point.progress_mm) * 1_000 - behind_um;
        Observed {
            status: VehicleStatus::Active,
            edge_index: point.edge_index,
            progress_mm: (at_um / 1_000) as u32,
            carry_um: (at_um % 1_000) as u16,
            speed_mm_s,
        }
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
        let to_entrance = approaching(ENTRANCE, 2_318, 217);
        let to_end = approaching(END, 2_318, 217);
        let classify = |before, after, exits| classify(before, after, exits, W16);
        assert_eq!(
            classify(to_entrance, observed(Active, ENTRANCE, 0), both),
            Exception::ParkingArrivalZero
        );
        assert_eq!(
            classify(to_end, observed(Completed, END, 0), both),
            Exception::RouteEndCompleted
        );
        // 无预约、未静止、带微米余数、位置不符或拍初非 Active 均不归入例外。
        let no_parking = Exits {
            parking_entrance: None,
            ..both
        };
        assert_eq!(
            classify(to_entrance, observed(Active, ENTRANCE, 0), no_parking),
            Exception::Unattributed
        );
        assert_eq!(
            classify(to_entrance, observed(Active, ENTRANCE, 1), both),
            Exception::Unattributed
        );
        let mut carried = observed(Active, ENTRANCE, 0);
        carried.carry_um = 1;
        assert_eq!(
            classify(to_entrance, carried, both),
            Exception::Unattributed
        );
        assert_eq!(
            classify(to_end, observed(Active, END, 0), both),
            Exception::Unattributed
        );
        let completed_before = Observed {
            status: Completed,
            ..to_end
        };
        assert_eq!(
            classify(completed_before, observed(Completed, END, 0), both),
            Exception::Unattributed
        );
        assert_eq!(
            classify(to_entrance, observed(Active, ENTRANCE, 0), Exits::default()),
            Exception::Unattributed
        );
    }

    #[test]
    fn terminal_zero_absorbs_at_most_one_tick_of_overrun() {
        let both = Exits {
            route_end: Some(END),
            parking_entrance: Some(ENTRANCE),
        };
        let arrived = observed(VehicleStatus::Active, ENTRANCE, 0);
        let at_entrance = |before| classify(before, arrived, both, W16);
        // W3 实测的到位拍形态（3b4d9f47）：16 ms 下拍初在入口前 0.016–2.318 mm、
        // 速度 108–217 mm/s；33 ms 下 3.077–6.934 mm、222–311 mm/s。
        for (behind_um, speed) in [(16, 108), (2_318, 217), (3_000, 240)] {
            assert_eq!(
                at_entrance(approaching(ENTRANCE, behind_um, speed)),
                Exception::ParkingArrivalZero,
                "{behind_um} µm / {speed} mm/s"
            );
        }
        let w33 = Motion { dt_ms: 33, ..W16 };
        for (behind_um, speed) in [(3_077, 222), (6_934, 311)] {
            assert_eq!(
                classify(approaching(ENTRANCE, behind_um, speed), arrived, both, w33),
                Exception::ParkingArrivalZero,
                "33 ms: {behind_um} µm / {speed} mm/s"
            );
        }
        // 边界：d = 0 时 (v−1)²·1000 ≤ 2·6000·16·v，193 满足、194 不满足。
        assert_eq!(
            at_entrance(approaching(ENTRANCE, 0, 193)),
            Exception::ParkingArrivalZero
        );
        assert_eq!(
            at_entrance(approaching(ENTRANCE, 0, 194)),
            Exception::Unattributed
        );
        // 以 13.9 m/s 冲到入口后一拍清零：越程远超一拍，不归入具名例外。
        assert_eq!(
            at_entrance(approaching(ENTRANCE, 222_400, 13_900)),
            Exception::Unattributed
        );
        // 拍初在另一 route occurrence 或已越过入口，均不可归因。
        let mut other_edge = approaching(ENTRANCE, 2_318, 217);
        other_edge.edge_index = 0;
        assert_eq!(at_entrance(other_edge), Exception::Unattributed);
        let mut beyond = approaching(ENTRANCE, 0, 100);
        beyond.carry_um = 1;
        assert_eq!(at_entrance(beyond), Exception::Unattributed);
    }
}
