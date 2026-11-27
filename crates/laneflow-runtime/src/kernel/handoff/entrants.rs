//! 从已提交的普通路线和当拍车身位置，证明外部横移候选两侧的潜在进入者能否停车。
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{BoundedDistance, StationAddress, StationCandidates};
use thiserror::Error;

use super::braking::{service_stop_distance_um, stop_from_cursor_mm};
use crate::kernel::state::WorldState;
use crate::kernel::tables::{CompiledRoute, distance_to_occurrence_progress};
use crate::kernel::tick::MotionReach;
use crate::{VehicleHandle, VehicleState, VehicleStatus};

/// 一辆可能妨碍外部横移候选的已提交车辆。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteHandoffEntrant {
    /// 本世界中的活动车辆。
    pub vehicle: VehicleHandle,
    /// 从已提交整数毫米车头沿自身路线到区间入口的毫米数。车身外包触及时为零。
    pub distance_mm: u32,
    /// 当前占用或沿本车路线接近的证明结果。
    pub reason: RouteHandoffEntrantReason,
}

/// 不能证明外部横移候选可由该车让行的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteHandoffEntrantReason {
    /// 整数毫米车身触及该侧的物理闭区间，车尾越过闭端所在毫米后清空。
    Occupied,
    /// 前方区间的距离不足以按服务减速度留出反应时间和最小间距，或停止距离未证。
    CannotStop {
        /// 从与 `distance_mm` 相同的整数车头原点到预计停点所需的毫米数，向上取整。
        /// 缺少有效制动参数或行程无法表示时为 None。
        required_mm: Option<u64>,
        /// 本拍运动上界也未排除到达；单拍不可达的车辆仍可能需要提前制动。
        may_reach_this_tick: bool,
    },
}

/// 按世界提交的车辆更新顺序，各侧取首个占用或不能证明安全停车的进入者。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteHandoffEntrants {
    /// 顺序为源、目标。各侧独立判定，同一车辆可能阻挡两侧。
    pub side_blockers: [Option<RouteHandoffEntrant>; 2],
    /// 已访问的 live 车辆槽、边地址查询及额外路线距离分段的合计。
    pub work: u32,
}

/// 进入者检查未能完成。已完成的部分不能用作通行许可。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffEntrantError {
    /// 剩余预算不足以执行下一次查询。
    #[error("进入者检查预算不足，已使用 {work} 项")]
    Budget {
        /// 已完成工作量。
        work: u32,
    },
    /// 已提交的车辆、车型或已编译路线缺失，或无法表示所需距离。
    #[error("进入者检查缺少完整状态或距离，已使用 {work} 项")]
    Invariant {
        /// 已完成工作量。
        work: u32,
    },
}

struct Work {
    used: u32,
    limit: u32,
}

impl Work {
    fn charge(&mut self, n: u32) -> Result<(), RouteHandoffEntrantError> {
        if n > self.limit - self.used {
            return Err(RouteHandoffEntrantError::Budget { work: self.used });
        }
        self.used += n;
        Ok(())
    }

    fn invariant(&self) -> RouteHandoffEntrantError {
        RouteHandoffEntrantError::Invariant { work: self.used }
    }
}

struct Approach<'a> {
    route: &'a CompiledRoute,
    vehicle: VehicleState,
    required_mm: Option<u64>,
    reach: Option<MotionReach>,
}

// 物理边的所有出现按路线下标相邻。取前后最近两次即可：更早的一次离车尾更远，
// 更晚的一次离车头更远。区间已在当前边身后时仍须保留下一圈的入口。
fn adjacent_occurrences(
    index: &[(LaneEdgeOrdinal, u32)],
    edge: LaneEdgeOrdinal,
    cursor: u32,
    front_mm: u32,
    [lo, hi]: [u32; 2],
) -> (Option<u32>, Option<u32>) {
    let slot = index.partition_point(|&(e, i)| (e.raw(), i) < (edge.raw(), cursor));
    let at = |slot: usize| index.get(slot).filter(|(e, _)| *e == edge).map(|&(_, i)| i);
    let mut behind = slot.checked_sub(1).and_then(at);
    let mut ahead = at(slot);
    if ahead == Some(cursor) {
        if front_mm >= hi {
            behind = Some(cursor);
        }
        if front_mm > lo {
            ahead = at(slot + 1);
        }
    }
    (behind, ahead)
}

/// 沿已编译路线求局部距离。额外跨过的每个完整分段在遍历前扣费。
fn distance_mm(
    route: &CompiledRoute,
    from: (u32, u32),
    to: (u32, u32),
    work: &mut Work,
) -> Result<BoundedDistance, RouteHandoffEntrantError> {
    let first = from.0 as usize;
    let last = to.0 as usize;
    let segments = &route.occurrence_segments;
    let start = segments.get(first).ok_or_else(|| work.invariant())?;
    let end = segments.get(last).ok_or_else(|| work.invariant())?;
    let spanned = end.checked_sub(*start).ok_or_else(|| work.invariant())?;
    work.charge(spanned.saturating_sub(1))?;
    distance_to_occurrence_progress(
        segments,
        &route.occurrence_offsets,
        &route.segment_totals,
        first,
        from.1,
        last,
        to.1,
    )
    .ok_or_else(|| work.invariant())
}

impl Approach<'_> {
    fn at(
        &self,
        address: StationAddress,
        work: &mut Work,
    ) -> Result<Option<RouteHandoffEntrant>, RouteHandoffEntrantError> {
        work.charge(1)?;
        let state = self.vehicle;
        let cursor = state.route_edge_index;
        let front = (cursor, state.progress_mm);
        let [lo, hi] = address.progress_mm;
        let current = self
            .route
            .edges
            .get(cursor as usize)
            .ok_or_else(|| work.invariant())?;
        let occupied = || {
            Some(RouteHandoffEntrant {
                vehicle: state.handle,
                distance_mm: 0,
                reason: RouteHandoffEntrantReason::Occupied,
            })
        };
        if *current == address.edge && (lo..=hi).contains(&state.progress_mm) {
            return Ok(occupied());
        }
        let (behind, ahead) = adjacent_occurrences(
            &self.route.edge_occurrence_index,
            address.edge,
            cursor,
            state.progress_mm,
            [lo, hi],
        );
        if let Some(index) = behind
            && let BoundedDistance::Finite(mm) = distance_mm(self.route, (index, hi), front, work)?
            && mm <= state.length_mm
        {
            return Ok(occupied());
        }
        let Some(index) = ahead else { return Ok(None) };
        let distance_mm = match distance_mm(self.route, front, (index, lo), work)? {
            BoundedDistance::Finite(mm) => mm,
            BoundedDistance::BeyondFinite => {
                // 已知整数路长至少为 u32::MAX + 1 毫米。停车界使用同一车头原点。
                if self
                    .required_mm
                    .is_some_and(|stop| stop <= u64::from(u32::MAX) + 1)
                {
                    return Ok(None);
                }
                return Err(work.invariant());
            }
        };
        // 前杠在前驱边末端时，同一接缝也属于后一边从零开始的闭区间。
        if distance_mm == 0 {
            return Ok(occupied());
        }
        if self
            .required_mm
            .is_some_and(|stop| u64::from(distance_mm) >= stop)
        {
            return Ok(None);
        }
        // MotionReach 从实际车头起算。距离向下取整，避免排除仍可能在本拍触及的进入者。
        let reach_distance_mm = distance_mm - u32::from(state.carry_um != 0);
        let may_reach_this_tick = self.reach.is_none_or(|r| !r.excludes(reach_distance_mm));
        Ok(Some(RouteHandoffEntrant {
            vehicle: state.handle,
            distance_mm,
            reason: RouteHandoffEntrantReason::CannotStop {
                required_mm: self.required_mm,
                may_reach_this_tick,
            },
        }))
    }

    fn on_side(
        &self,
        side: &StationCandidates,
        work: &mut Work,
    ) -> Result<Option<RouteHandoffEntrant>, RouteHandoffEntrantError> {
        let mut nearest: Option<RouteHandoffEntrant> = None;
        for address in side.addresses() {
            if let Some(blocker) = self.at(address, work)? {
                if blocker.reason == RouteHandoffEntrantReason::Occupied {
                    return Ok(Some(blocker));
                }
                if nearest.is_none_or(|other| blocker.distance_mm < other.distance_mm) {
                    nearest = Some(blocker);
                }
            }
        }
        Ok(nearest)
    }
}

pub(super) fn check_entrants(
    state: &WorldState,
    sides: [Option<&StationCandidates>; 2],
    work_limit: u32,
) -> Result<RouteHandoffEntrants, RouteHandoffEntrantError> {
    check_other_entrants(state.read_view(), sides, work_limit, None, &[])
}

pub(super) fn check_other_entrants(
    state: crate::kernel::phase::StepReadView<'_>,
    sides: [Option<&StationCandidates>; 2],
    work_limit: u32,
    owner: Option<VehicleHandle>,
    protections: &[super::execution::HandoffProtection],
) -> Result<RouteHandoffEntrants, RouteHandoffEntrantError> {
    let mut work = Work {
        used: 0,
        limit: work_limit,
    };
    let mut side_blockers = [None; 2];
    let dt = state.binding.config.fixed_delta_time_ms();
    let relations = state.binding.revision.traffic().relations();
    for &handle in &state.committed.live_order {
        if (0..2).all(|i| {
            (sides[i].is_none() && (i != 1 || protections.is_empty())) || side_blockers[i].is_some()
        }) {
            break;
        }
        work.charge(1)?;
        if Some(handle) == owner {
            continue;
        }
        let v = state
            .committed
            .vehicles
            .state(handle)
            .ok_or_else(|| work.invariant())?;
        if v.status != VehicleStatus::Active {
            continue;
        }
        let route = state
            .compiled_route(v.route)
            .ok_or_else(|| work.invariant())?;
        let profile = relations
            .vehicle_profile(v.profile)
            .ok_or_else(|| work.invariant())?;
        let approach = Approach {
            route,
            vehicle: v,
            required_mm: stop_from_cursor_mm(
                service_stop_distance_um(
                    v.speed_mm_s,
                    dt,
                    profile.comfort_decel(),
                    profile.min_gap_mm(),
                ),
                v.carry_um,
            ),
            reach: MotionReach::from_tick(v.speed_mm_s, profile.max_accel(), dt as f32 / 1_000.0),
        };
        for (blocker, side) in side_blockers.iter_mut().zip(sides) {
            if blocker.is_none()
                && let Some(side) = side
            {
                *blocker = approach.on_side(side, &mut work)?;
            }
        }
        if side_blockers[1].is_none() {
            for p in protections.iter().filter(|p| p.role == 4) {
                if let Some(blocker) = approach.at(
                    StationAddress {
                        edge: p.edge,
                        progress_mm: [p.start_mm, p.end_mm],
                    },
                    &mut work,
                )? {
                    side_blockers[1] = Some(blocker);
                    break;
                }
            }
        }
    }
    Ok(RouteHandoffEntrants {
        side_blockers,
        work: work.used,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::placement_admission_tests::{install_dt, register_named, revision};
    use crate::{RouteHandle, TrafficWorld, VehicleSpawnInput};
    use laneflow_compiler::{LaneEdgeInput, LaneEdgeReference};
    use laneflow_static_contract::VehicleProfileOrdinal;

    fn cyclic_route() -> (TrafficWorld, RouteHandle) {
        let ns = "runtime/handoff-entrants";
        let revision = revision(ns, |module| {
            for (key, next) in [("a", "b"), ("b", "c"), ("c", "a")] {
                module
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: key,
                        length_meters: 10.0,
                        speed_limit_meters_per_second: 10.0,
                        successors: &[LaneEdgeReference::local(next)],
                    })
                    .unwrap();
            }
        });
        let mut world = install_dt(revision, 100);
        let route = register_named(&mut world, ns, &["a", "b", "c", "a"]);
        (world, route)
    }

    #[test]
    fn a_front_on_the_predecessor_touches_the_successor_even_with_no_stopping_gap() {
        let (mut world, route) = cyclic_route();
        let v = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                10_000,
                0,
            ))
            .unwrap();
        let route = world.state.compiled_route(route).unwrap();
        let vehicle = world.state.vehicle_state(v).unwrap();
        assert_eq!(
            (
                vehicle.route_edge_index,
                vehicle.progress_mm,
                vehicle.carry_um
            ),
            (0, 10_000, 0)
        );
        let mut approach = Approach {
            route,
            vehicle,
            required_mm: stop_from_cursor_mm(service_stop_distance_um(0, 100, 2.0, 0), 0),
            reach: MotionReach::from_tick(0, 1.5, 0.1),
        };
        assert_eq!(approach.required_mm, Some(0));
        let mut work = Work { used: 0, limit: 6 };
        let address = StationAddress {
            edge: route.edges[1],
            progress_mm: [0, 100],
        };
        assert_eq!(
            approach.at(address, &mut work).unwrap(),
            Some(RouteHandoffEntrant {
                vehicle: v,
                distance_mm: 0,
                reason: RouteHandoffEntrantReason::Occupied,
            })
        );
        approach.vehicle.progress_mm -= 1;
        approach.vehicle.carry_um = 999;
        approach.required_mm = stop_from_cursor_mm(Some(0), 999);
        assert_eq!(approach.at(address, &mut work).unwrap(), None);
        approach.required_mm = stop_from_cursor_mm(Some(2_000_000), 999);
        let blocker = approach.at(address, &mut work).unwrap().unwrap();
        assert_eq!(blocker.distance_mm, 1);
        assert_eq!(
            blocker.reason,
            RouteHandoffEntrantReason::CannotStop {
                required_mm: Some(2_001),
                may_reach_this_tick: true,
            }
        );
        // 物理可达上界 57.5 mm。从整数原点距入口 58 mm，正余数时本拍仍可能触及。
        approach.vehicle.progress_mm = 10_000 - 58;
        for (carry_um, expected) in [(0, false), (999, true)] {
            approach.vehicle.carry_um = carry_um;
            approach.required_mm = stop_from_cursor_mm(Some(2_000_000), carry_um);
            let blocker = approach.at(address, &mut work).unwrap().unwrap();
            assert_eq!(blocker.distance_mm, 58);
            assert!(
                matches!(blocker.reason, RouteHandoffEntrantReason::CannotStop {
                may_reach_this_tick, ..
            } if may_reach_this_tick == expected)
            );
        }
    }

    #[test]
    fn a_tail_clears_the_closed_end_before_checking_the_next_lap_stop_point() {
        let (mut world, route) = cyclic_route();
        let v = world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 6_000, 0)
                    .with_open_entrance(),
            )
            .unwrap();
        let route = world.state.compiled_route(route).unwrap();
        let mut approach = Approach {
            route,
            vehicle: world.state.vehicle_state(v).unwrap(),
            required_mm: Some(25_000),
            reach: MotionReach::from_tick(0, 1.5, 0.1),
        };
        let mut work = Work { used: 0, limit: 12 };
        let mut address = StationAddress {
            edge: route.edges[0],
            progress_mm: [1_000, 1_500],
        };
        for carry_um in [0, 1, 999] {
            approach.vehicle.carry_um = carry_um;
            assert_eq!(
                approach.at(address, &mut work).unwrap().unwrap().reason,
                RouteHandoffEntrantReason::Occupied
            );
        }
        // 尾部进入下一毫米。本圈已清空，从整数车头到下一圈的入口仍有 24,999 mm。
        approach.vehicle.progress_mm += 1;
        approach.vehicle.carry_um = 1;
        approach.required_mm = stop_from_cursor_mm(Some(24_999_000), 1);
        let blocked = approach.at(address, &mut work).unwrap().unwrap();
        assert_eq!(blocked.distance_mm, 24_999);
        assert_eq!(
            blocked.reason,
            RouteHandoffEntrantReason::CannotStop {
                required_mm: Some(25_000),
                may_reach_this_tick: false,
            }
        );
        // 相等允许；无制动参数时仍不能证明让行。
        approach.required_mm = stop_from_cursor_mm(Some(24_998_999), 1);
        assert_eq!(approach.required_mm, Some(u64::from(blocked.distance_mm)));
        assert_eq!(approach.at(address, &mut work).unwrap(), None);
        approach.required_mm = None;
        assert_eq!(
            approach.at(address, &mut work).unwrap().unwrap().reason,
            RouteHandoffEntrantReason::CannotStop {
                required_mm: None,
                may_reach_this_tick: false
            }
        );

        // 前杠已跨两条短边。车尾落在 a 的闭端毫米中仍然占用。
        address.progress_mm = [9_000, 9_700];
        approach.vehicle.route_edge_index = 2;
        approach.vehicle.progress_mm = 100;
        approach.vehicle.length_mm = 10_400;
        for carry_um in [0, 999] {
            approach.vehicle.carry_um = carry_um;
            assert_eq!(
                approach.at(address, &mut work).unwrap().unwrap().reason,
                RouteHandoffEntrantReason::Occupied
            );
        }
        approach.vehicle.progress_mm += 1;
        assert_eq!(
            approach
                .at(address, &mut work)
                .unwrap()
                .unwrap()
                .distance_mm,
            18_899
        );
        assert_eq!(work.used, 9);
        work.limit = work.used;
        assert_eq!(
            approach.at(address, &mut work),
            Err(RouteHandoffEntrantError::Budget { work: 9 })
        );
    }
}
