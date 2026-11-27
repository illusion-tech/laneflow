//! 从已提交的普通路线和当拍车身位置，证明外部横移候选两侧的潜在进入者能否停车。
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{BoundedDistance, StationAddress, StationCandidates};
use thiserror::Error;

use crate::kernel::lane_change::service_stop_distance_um;
use crate::kernel::state::WorldState;
use crate::kernel::tables::{CompiledRoute, distance_to_occurrence_progress};
use crate::kernel::tick::MotionReach;
use crate::{VehicleHandle, VehicleState, VehicleStatus};

/// 一辆可能妨碍外部横移候选的已提交车辆。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteHandoffEntrant {
    /// 本世界中的活动车辆。
    pub vehicle: VehicleHandle,
    /// 车头沿自身路线到该侧最近区间入口的距离（微米）。车身已经占用时为零。
    pub distance_um: u64,
    /// 当前占用或沿本车路线接近的证明结果。
    pub reason: RouteHandoffEntrantReason,
}

/// 不能证明外部横移候选可由该车让行的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteHandoffEntrantReason {
    /// 按已提交的车身长度、前杠与微米余量，车辆触及该侧某个物理闭区间。
    Occupied,
    /// 前方区间的距离不足以按服务减速度留出反应时间和最小间距，或停止距离未证。
    CannotStop {
        /// 该进入者自身所需的停止距离（微米）；未证时为 None。
        required_um: Option<u64>,
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
    required_um: Option<u64>,
    reach: Option<MotionReach>,
}

// 物理边的所有出现按路线下标相邻。取前后最近两次即可：更早的一次离车尾更远，
// 更晚的一次离车头更远。区间已在当前边身后时仍须保留下一圈的入口。
fn adjacent_occurrences(
    index: &[(LaneEdgeOrdinal, u32)],
    edge: LaneEdgeOrdinal,
    cursor: u32,
    front_um: u64,
    [lo, hi]: [u32; 2],
) -> (Option<u32>, Option<u32>) {
    let slot = index.partition_point(|&(e, i)| (e.raw(), i) < (edge.raw(), cursor));
    let at = |slot: usize| index.get(slot).filter(|(e, _)| *e == edge).map(|&(_, i)| i);
    let mut behind = slot.checked_sub(1).and_then(at);
    let mut ahead = at(slot);
    if ahead == Some(cursor) {
        if front_um >= um(hi) {
            behind = Some(cursor);
        }
        if front_um > um(lo) {
            ahead = at(slot + 1);
        }
    }
    (behind, ahead)
}

fn um(mm: u32) -> u64 {
    u64::from(mm) * 1_000
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
        let carry = u64::from(state.carry_um);
        let front_um = um(state.progress_mm) + carry;
        let [lo, hi] = address.progress_mm;
        let current = self
            .route
            .edges
            .get(cursor as usize)
            .ok_or_else(|| work.invariant())?;
        let occupied = || {
            Some(RouteHandoffEntrant {
                vehicle: state.handle,
                distance_um: 0,
                reason: RouteHandoffEntrantReason::Occupied,
            })
        };
        if *current == address.edge && (um(lo)..=um(hi)).contains(&front_um) {
            return Ok(occupied());
        }
        let (behind, ahead) = adjacent_occurrences(
            &self.route.edge_occurrence_index,
            address.edge,
            cursor,
            front_um,
            [lo, hi],
        );
        if let Some(index) = behind
            && let BoundedDistance::Finite(mm) = distance_mm(self.route, (index, hi), front, work)?
            && um(mm) + carry <= um(state.length_mm)
        {
            return Ok(occupied());
        }
        let Some(index) = ahead else { return Ok(None) };
        let mm = match distance_mm(self.route, front, (index, lo), work)? {
            BoundedDistance::Finite(mm) => mm,
            BoundedDistance::BeyondFinite => {
                // 首个越界的整毫米扣除最多 999 微米，仍比 u32::MAX 毫米远。
                if self.required_um.is_some_and(|stop| stop <= um(u32::MAX)) {
                    return Ok(None);
                }
                return Err(work.invariant());
            }
        };
        let distance_um = um(mm).checked_sub(carry).ok_or_else(|| work.invariant())?;
        // 前杠在前驱边末端时，同一接缝也属于后一边从零开始的闭区间。
        if distance_um == 0 {
            return Ok(occupied());
        }
        if self.required_um.is_some_and(|stop| distance_um >= stop) {
            return Ok(None);
        }
        // 向下取整数毫米交给位移上界，只能多保留候选，不能把仍可能到达者排除。
        let may_reach_this_tick = self
            .reach
            .is_none_or(|r| !r.excludes((distance_um / 1_000) as u32));
        Ok(Some(RouteHandoffEntrant {
            vehicle: state.handle,
            distance_um,
            reason: RouteHandoffEntrantReason::CannotStop {
                required_um: self.required_um,
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
                if nearest.is_none_or(|other| blocker.distance_um < other.distance_um) {
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
    let mut work = Work {
        used: 0,
        limit: work_limit,
    };
    let mut side_blockers = [None; 2];
    let dt = state.binding.config.fixed_delta_time_ms();
    let relations = state.binding.revision.traffic().relations();
    for &handle in &state.committed.live_order {
        if (0..2).all(|i| sides[i].is_none() || side_blockers[i].is_some()) {
            break;
        }
        work.charge(1)?;
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
            required_um: service_stop_distance_um(
                v.speed_mm_s,
                dt,
                profile.comfort_decel(),
                profile.min_gap_mm(),
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
            required_um: service_stop_distance_um(0, 100, 2.0, 0),
            reach: MotionReach::from_tick(0, 1.5, 0.1),
        };
        assert_eq!(approach.required_um, Some(0));
        let mut work = Work { used: 0, limit: 3 };
        let address = StationAddress {
            edge: route.edges[1],
            progress_mm: [0, 100],
        };
        assert_eq!(
            approach.at(address, &mut work).unwrap(),
            Some(RouteHandoffEntrant {
                vehicle: v,
                distance_um: 0,
                reason: RouteHandoffEntrantReason::Occupied,
            })
        );
        approach.vehicle.progress_mm -= 1;
        approach.vehicle.carry_um = 999;
        assert_eq!(approach.at(address, &mut work).unwrap(), None);
        approach.required_um = Some(2_000_000);
        let blocker = approach.at(address, &mut work).unwrap().unwrap();
        assert_eq!(blocker.distance_um, 1);
        assert_eq!(
            blocker.reason,
            RouteHandoffEntrantReason::CannotStop {
                required_um: Some(2_000_000),
                may_reach_this_tick: true,
            }
        );
    }

    #[test]
    fn a_cleared_tail_still_checks_the_next_occurrence_one_micrometre_inside_the_stop_distance() {
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
            required_um: Some(25_000_000),
            reach: MotionReach::from_tick(0, 1.5, 0.1),
        };
        let mut work = Work { used: 0, limit: 8 };
        let mut address = StationAddress {
            edge: route.edges[0],
            progress_mm: [1_000, 1_500],
        };
        assert_eq!(
            approach.at(address, &mut work).unwrap().unwrap().reason,
            RouteHandoffEntrantReason::Occupied
        );
        approach.vehicle.carry_um = 1;
        // 尾部已过本次出现 1 微米；下一圈入口仍需检查，且制动差 1 微米。
        let blocked = approach.at(address, &mut work).unwrap().unwrap();
        assert_eq!(blocked.distance_um, 24_999_999);
        assert_eq!(
            blocked.reason,
            RouteHandoffEntrantReason::CannotStop {
                required_um: Some(25_000_000),
                may_reach_this_tick: false,
            }
        );
        // 相等允许；无制动参数时仍不能证明让行。
        approach.required_um = Some(blocked.distance_um);
        assert_eq!(approach.at(address, &mut work).unwrap(), None);
        approach.required_um = None;
        assert_eq!(
            approach.at(address, &mut work).unwrap().unwrap().reason,
            RouteHandoffEntrantReason::CannotStop {
                required_um: None,
                may_reach_this_tick: false
            }
        );

        // 前杠已跨两条短边。车尾恰好与 a 上的端点相接，下一微米清空。
        address.progress_mm = [9_000, 9_700];
        approach.vehicle.route_edge_index = 2;
        approach.vehicle.progress_mm = 100;
        approach.vehicle.length_mm = 10_400;
        approach.vehicle.carry_um = 0;
        assert_eq!(
            approach.at(address, &mut work).unwrap().unwrap().reason,
            RouteHandoffEntrantReason::Occupied
        );
        approach.vehicle.carry_um = 1;
        assert_eq!(
            approach
                .at(address, &mut work)
                .unwrap()
                .unwrap()
                .distance_um,
            18_899_999
        );
        assert_eq!(work.used, 6);
        work.limit = work.used;
        assert_eq!(
            approach.at(address, &mut work),
            Err(RouteHandoffEntrantError::Budget { work: 6 })
        );
    }
}
