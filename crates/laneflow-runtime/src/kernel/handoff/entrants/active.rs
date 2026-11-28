use super::{
    Approach, RouteHandoffEntrant, RouteHandoffEntrantError, RouteHandoffEntrantReason, Work,
    adjacent_occurrences, distance_mm,
};
use crate::kernel::{
    handoff::{
        LocatedTrajectory,
        execution::{
            ActiveRouteHandoff, PreparedRouteHandoff, committed_position, matches_entry,
            position_on, remaining_sweep, swept,
        },
        sample::LocatedSample,
    },
    phase::StepReadView,
};
use laneflow_static_network::{
    BoundedDistance, SharedNetworkRevision, StationAddress, StationCandidates,
    vehicle::GeometryWorkLimits,
};

/// 本车当前车身、已获准的剩余扫过，以及恢复普通行驶的目标前杠。
/// 每辆进入者只定位一次；各个被查询的地址共用这些证据。
pub(super) struct ActiveApproach<'a> {
    path: LocatedTrajectory<'a>,
    current: LocatedSample,
    remaining_sweep: [Option<StationCandidates>; 2],
    travel_um: u64,
    remaining_mm: u64,
    terminal_front: Option<(u32, u32)>,
    limits: GeometryWorkLimits,
}

// 同一编制车道的前驱末端和后继零点位于同一个物理切点。
fn touches(revision: &SharedNetworkRevision, a: StationAddress, b: StationAddress) -> bool {
    let [alo, ahi] = a.progress_mm;
    let [blo, bhi] = b.progress_mm;
    let seam = |left: StationAddress, hi, right: StationAddress, lo| {
        lo == 0
            && revision.traffic().lane_lengths_millimetres()[left.edge.index()] == hi
            && revision.station_mapping().preceding_lane_edge(right.edge) == Some(left.edge)
    };
    if a.edge == b.edge {
        alo <= bhi && blo <= ahi
    } else {
        seam(a, ahi, b, blo) || seam(b, bhi, a, alo)
    }
}

impl<'a> ActiveApproach<'a> {
    pub(super) fn new(
        world: StepReadView<'a>,
        record: &ActiveRouteHandoff,
        selected: Option<&PreparedRouteHandoff>,
        approach: &Approach<'a>,
        work: &mut Work,
    ) -> Result<Self, RouteHandoffEntrantError> {
        if record.vehicle != approach.vehicle.handle {
            return Err(work.invariant());
        }
        work.charge(1)?;
        let (path, length_lower_um) = record
            .entrant_path(world, approach.route, approach.vehicle, selected)
            .ok_or_else(|| work.invariant())?;
        let limits = GeometryWorkLimits {
            tracks: record.tracks,
            sweep: record.sweep,
        };
        work.charge(1)?;
        let (current, _, front) =
            position_on(&path, record.rear_travel_um, limits, selected.is_none())
                .ok_or_else(|| work.invariant())?;
        let matches = if selected.is_some() {
            record.rear_travel_um == 0 && matches_entry(approach.vehicle, &current)
        } else {
            committed_position(approach.vehicle, &current, front)
        };
        if !matches {
            return Err(work.invariant());
        }
        work.charge(1)?;
        let remaining_sweep =
            remaining_sweep(&path, current.rear, limits.sweep).ok_or_else(|| work.invariant())?;
        let remaining_mm = length_lower_um.saturating_sub(record.rear_travel_um) / 1000;
        let terminal_front = if approach.required_mm.is_none_or(|stop| stop > remaining_mm) {
            work.charge(1)?;
            Some(
                path.terminal_front(limits.sweep)
                    .ok_or_else(|| work.invariant())?
                    .2,
            )
        } else {
            None
        };
        Ok(Self {
            path,
            current,
            remaining_sweep,
            travel_um: record.rear_travel_um,
            remaining_mm,
            terminal_front,
            limits,
        })
    }

    /// 只有完整前缀的连续扫过与地址分离，才可把该行程作为首次触及的下界。
    fn prefix_clear(
        &self,
        mm: u64,
        address: StationAddress,
        work: &mut Work,
    ) -> Result<bool, RouteHandoffEntrantError> {
        let Some(total) = mm
            .checked_mul(1000)
            .and_then(|um| self.travel_um.checked_add(um))
        else {
            return Err(work.invariant());
        };
        work.charge(1)?;
        let bounds = self
            .path
            .revision
            .handoff_travel_after_motion(
                &self.path.query,
                self.path.window,
                total / 1000,
                (total % 1000) as u16,
                1,
                self.limits.tracks,
            )
            .bounds;
        let Ok(bounds) = bounds else { return Ok(false) };
        work.charge(1)?;
        let sides = swept(
            &self.path,
            self.current.rear.hull(bounds.rear),
            self.limits.sweep,
        );
        Ok(sides.is_some_and(|s| !self.swept_touches(&s, address)))
    }

    fn swept_touches(
        &self,
        sides: &[Option<StationCandidates>; 2],
        address: StationAddress,
    ) -> bool {
        sides
            .iter()
            .flatten()
            .flat_map(StationCandidates::addresses)
            .any(|a| touches(self.path.revision, a, address))
    }

    pub(super) fn at(
        &self,
        approach: &Approach<'_>,
        address: StationAddress,
        work: &mut Work,
    ) -> Result<Option<RouteHandoffEntrant>, RouteHandoffEntrantError> {
        if self
            .current
            .footprint
            .sides
            .iter()
            .flatten()
            .flat_map(|s| s.enclosure.addresses())
            .any(|a| touches(self.path.revision, a, address))
        {
            return Ok(Some(RouteHandoffEntrant {
                vehicle: approach.vehicle.handle,
                distance_mm: 0,
                reason: RouteHandoffEntrantReason::Occupied,
            }));
        }
        if self.swept_touches(&self.remaining_sweep, address) {
            let mut high = approach
                .required_mm
                .unwrap_or(u64::MAX)
                .min(self.remaining_mm);
            let mut low = 0;
            if high != 0 && self.prefix_clear(high, address, work)? {
                low = high;
            } else {
                while high - low > 1 {
                    let mid = low + (high - low) / 2;
                    if self.prefix_clear(mid, address, work)? {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
            }
            return approach.braking_blocker(low, work);
        }
        let Some(front) = self.terminal_front else {
            return Ok(None);
        };
        // 终点以后的前杠沿目标普通边前进，到下一次尚未获准的横移为止。
        let [lo, hi] = address.progress_mm;
        let route = approach.route;
        let (index, entry) =
            if route.edges[front.0 as usize] == address.edge && (lo..=hi).contains(&front.1) {
                (front.0, front.1)
            } else {
                let (_, ahead) = adjacent_occurrences(
                    &route.edge_occurrence_index,
                    address.edge,
                    front.0,
                    front.1,
                    [lo, hi],
                );
                let Some(index) = ahead else { return Ok(None) };
                (index, lo)
            };
        let windows = route.handoff_windows();
        if windows
            .get(windows.partition_point(|(i, _)| *i < front.0))
            .is_some_and(|(i, _)| *i < index)
        {
            return Ok(None);
        }
        match distance_mm(route, front, (index, entry), work)? {
            BoundedDistance::Finite(mm) => {
                approach.braking_blocker(self.remaining_mm.saturating_add(u64::from(mm)), work)
            }
            BoundedDistance::BeyondFinite
                if approach
                    .required_mm
                    .is_some_and(|stop| stop <= u64::from(u32::MAX) + 1) =>
            {
                Ok(None)
            }
            BoundedDistance::BeyondFinite => Err(work.invariant()),
        }
    }
}
