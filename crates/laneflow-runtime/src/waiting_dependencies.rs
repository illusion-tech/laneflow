//! Waiting 容量视图、反向依赖阈值与候选图事务。

use crate::waiting_graph::WaitingGraph;
use crate::{StepError, TrafficWorld, VehicleHandle};
use laneflow_static_contract::WaitingZoneOrdinal;
use std::num::NonZeroU32;

fn reserve<T>(values: &mut Vec<T>, count: usize) -> Result<(), StepError> {
    crate::conflict_tick::reserve(values, count)
}
fn encode(index: usize) -> Result<NonZeroU32, StepError> {
    u32::try_from(index)
        .ok()
        .and_then(|value| value.checked_add(1))
        .and_then(NonZeroU32::new)
        .ok_or(StepError::ConflictInvariantViolation)
}
fn decode(index: NonZeroU32) -> usize {
    index.get() as usize - 1
}

#[derive(Clone, Copy, Default)]
struct OwnerIndex {
    node: Option<NonZeroU32>,
    holds: [Option<NonZeroU32>; 2],
}

#[derive(Clone, Copy)]
struct Zone {
    ordinal: WaitingZoneOrdinal,
    node: usize,
    count: u32,
    maximum: u32,
    used: u64,
    watcher_start: usize,
    watcher_end: usize,
    watcher_cursor: usize,
}

#[derive(Clone, Copy)]
struct Hold {
    owner: VehicleHandle,
    zone: usize,
    edge: usize,
    active: bool,
    length: u32,
    gap: u32,
    dependency_start: usize,
    dependency_end: usize,
}

#[derive(Clone, Copy)]
struct Dependency {
    hold: usize,
    zone: usize,
    edge: usize,
    storage: u32,
    allowance: i64,
    blocked: bool,
}

#[derive(Clone, Copy)]
struct Pending {
    hold: usize,
    count: u32,
    used: u64,
    cursor: usize,
}

#[derive(Default)]
pub(crate) struct WaitingDependencies {
    graph: WaitingGraph,
    owner_index: Vec<OwnerIndex>,
    zone_index: Vec<Option<NonZeroU32>>,
    plan_index: Vec<Option<NonZeroU32>>,
    zones: Vec<Zone>,
    holds: Vec<Hold>,
    dependencies: Vec<Dependency>,
    watchers: Vec<usize>,
    changes: Vec<(bool, usize)>,
    pending: Option<Pending>,
}

impl WaitingDependencies {
    pub(crate) fn abort(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.graph.clear();
        for zone in &self.zones {
            self.zone_index[zone.ordinal.index()] = None;
        }
        for hold in &self.holds {
            self.owner_index[hold.owner.index() as usize] = OwnerIndex::default();
        }
        self.zones.clear();
        self.holds.clear();
        self.dependencies.clear();
        self.watchers.clear();
        self.plan_index.clear();
        self.changes.clear();
        self.pending = None;
    }

    fn prepare_indices(
        &mut self,
        owners: usize,
        zones: usize,
        plans: usize,
    ) -> Result<(), StepError> {
        if owners > self.owner_index.len() {
            let additional = owners - self.owner_index.len();
            reserve(&mut self.owner_index, additional)?;
            self.owner_index.resize(owners, OwnerIndex::default());
        }
        if zones > self.zone_index.len() {
            let additional = zones - self.zone_index.len();
            reserve(&mut self.zone_index, additional)?;
            self.zone_index.resize(zones, None);
        }
        reserve(&mut self.plan_index, plans)?;
        self.plan_index.resize(plans, None);
        Ok(())
    }

    fn zone(&mut self, ordinal: WaitingZoneOrdinal, maximum: u32) -> Result<usize, StepError> {
        if let Some(index) = self.zone_index[ordinal.index()] {
            return Ok(decode(index));
        }
        reserve(&mut self.zones, 1)?;
        let index = self.zones.len();
        let encoded = encode(index)?;
        let node = self.graph.node()?;
        self.zones.push(Zone {
            ordinal,
            node,
            maximum,
            count: 0,
            used: 0,
            watcher_start: 0,
            watcher_end: 0,
            watcher_cursor: 0,
        });
        self.zone_index[ordinal.index()] = Some(encoded);
        Ok(index)
    }

    fn hold(
        &mut self,
        owner: VehicleHandle,
        zone: usize,
        length: u32,
        gap: u32,
        active: bool,
        plan: Option<usize>,
    ) -> Result<usize, StepError> {
        let slot = owner.index() as usize;
        let node = match self.owner_index[slot].node {
            Some(index) => decode(index),
            None => {
                let index = self.graph.node()?;
                self.owner_index[slot].node = Some(encode(index)?);
                index
            }
        };
        let hold_slot = self.owner_index[slot]
            .holds
            .iter()
            .position(Option::is_none)
            .ok_or(StepError::WaitingInvariantViolation)?;
        reserve(&mut self.holds, 1)?;
        let index = self.holds.len();
        let encoded = encode(index)?;
        let edge = self.graph.edge(self.zones[zone].node, node, active)?;
        self.holds.push(Hold {
            owner,
            zone,
            edge,
            length,
            gap,
            active,
            dependency_start: self.dependencies.len(),
            dependency_end: self.dependencies.len(),
        });
        self.owner_index[slot].holds[hold_slot] = Some(encoded);
        if let Some(plan) = plan {
            self.plan_index[plan] = Some(encoded);
        }
        if active {
            let load = &mut self.zones[zone];
            load.used = load
                .used
                .checked_add(u64::from(length))
                .and_then(|value| {
                    value.checked_add(if load.count == 0 { 0 } else { u64::from(gap) })
                })
                .ok_or(StepError::WaitingInvariantViolation)?;
            load.count = load
                .count
                .checked_add(1)
                .ok_or(StepError::WaitingInvariantViolation)?;
        }
        Ok(index)
    }

    fn dependency(&mut self, hold: usize, zone: usize, storage: u32) -> Result<(), StepError> {
        reserve(&mut self.dependencies, 1)?;
        let value = self.holds[hold];
        let owner = decode(
            self.owner_index[value.owner.index() as usize]
                .node
                .expect("hold node"),
        );
        let edge = self.graph.edge(owner, self.zones[zone].node, false)?;
        self.dependencies.push(Dependency {
            hold,
            zone,
            edge,
            storage,
            allowance: i64::from(storage) - i64::from(value.length) - i64::from(value.gap),
            blocked: false,
        });
        self.holds[hold].dependency_end = self.dependencies.len();
        Ok(())
    }

    fn owner_holds(&self, owner: VehicleHandle, zone: usize, prospective: Option<usize>) -> bool {
        self.owner_index[owner.index() as usize]
            .holds
            .iter()
            .copied()
            .flatten()
            .any(|index| {
                let index = decode(index);
                let hold = self.holds[index];
                hold.owner == owner
                    && hold.zone == zone
                    && (hold.active || Some(index) == prospective)
            })
    }

    fn blocked(&self, dependency: Dependency, pending: Option<Pending>) -> bool {
        let zone = self.zones[dependency.zone];
        let (count, used) = pending
            .filter(|pending| self.holds[pending.hold].zone == dependency.zone)
            .map_or((zone.count, zone.used), |pending| {
                (pending.count, pending.used)
            });
        if count >= zone.maximum {
            return true;
        }
        if count == 0 {
            self.holds[dependency.hold].length > dependency.storage
        } else {
            dependency.allowance < 0 || used > dependency.allowance as u64
        }
    }

    fn finish_prepare(&mut self) -> Result<(), StepError> {
        reserve(&mut self.watchers, self.dependencies.len())?;
        for index in 0..self.dependencies.len() {
            let dependency = self.dependencies[index];
            let blocked = self.blocked(dependency, None);
            self.dependencies[index].blocked = blocked;
            let hold = self.holds[dependency.hold];
            self.graph.initialize(
                dependency.edge,
                blocked && hold.active && !self.owner_holds(hold.owner, dependency.zone, None),
            );
            self.watchers.push(index);
        }
        self.watchers.sort_unstable_by_key(|index| {
            let dependency = self.dependencies[*index];
            (
                self.zones[dependency.zone].ordinal,
                dependency.allowance,
                *index,
            )
        });
        for (position, index) in self.watchers.iter().copied().enumerate() {
            let zone = &mut self.zones[self.dependencies[index].zone];
            if zone.watcher_end == 0 {
                zone.watcher_start = position;
                zone.watcher_cursor = position;
            }
            zone.watcher_end = position + 1;
        }
        for zone in &mut self.zones {
            if zone.count != 0 {
                zone.watcher_cursor = zone.watcher_start
                    + self.watchers[zone.watcher_start..zone.watcher_end]
                        .partition_point(|index| self.dependencies[*index].blocked);
            }
        }
        if !self.graph.seal()? {
            return Err(StepError::WaitingInvariantViolation);
        }
        Ok(())
    }

    fn change(&mut self, edge: usize, active: bool) -> Result<(), StepError> {
        reserve(&mut self.changes, 1)?;
        self.changes.push((active, edge));
        Ok(())
    }

    pub(crate) fn stage(&mut self, plan: usize) -> Result<bool, StepError> {
        debug_assert!(self.pending.is_none());
        self.changes.clear();
        self.graph.begin();
        let result = self.stage_inner(plan);
        if !matches!(result, Ok(true)) {
            self.rollback();
        }
        result
    }

    fn stage_inner(&mut self, plan: usize) -> Result<bool, StepError> {
        let index = self
            .plan_index
            .get(plan)
            .copied()
            .flatten()
            .map(decode)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let hold = self.holds[index];
        let zone = self.zones[hold.zone];
        if hold.active {
            return Err(StepError::WaitingInvariantViolation);
        }
        let count = zone
            .count
            .checked_add(1)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let used = zone
            .used
            .checked_add(u64::from(hold.length))
            .and_then(|value| {
                value.checked_add(if zone.count == 0 {
                    0
                } else {
                    u64::from(hold.gap)
                })
            })
            .ok_or(StepError::WaitingInvariantViolation)?;
        if count > zone.maximum {
            return Err(StepError::WaitingInvariantViolation);
        }
        let mut pending = Pending {
            hold: index,
            count,
            used,
            cursor: zone.watcher_cursor,
        };
        self.change(hold.edge, true)?;
        // 所有旧/新 hold 共同决定自等待排除，删除边先于新增边。
        for owned in self.owner_index[hold.owner.index() as usize]
            .holds
            .into_iter()
            .flatten()
        {
            let owned = decode(owned);
            let value = self.holds[owned];
            if !value.active && owned != index {
                continue;
            }
            for dependency in value.dependency_start..value.dependency_end {
                let dependency = self.dependencies[dependency];
                let active = self.blocked(dependency, Some(pending))
                    && !self.owner_holds(hold.owner, dependency.zone, Some(index));
                self.change(dependency.edge, active)?;
            }
        }
        while pending.cursor < zone.watcher_end {
            let dependency = self.dependencies[self.watchers[pending.cursor]];
            if !self.blocked(dependency, Some(pending)) {
                break;
            }
            pending.cursor += 1;
            #[cfg(test)]
            crate::conflict::count_conflict_work(|work| work.wait_for_thresholds += 1);
            let dependent = self.holds[dependency.hold];
            self.change(
                dependency.edge,
                (dependent.active || dependency.hold == index)
                    && !self.owner_holds(dependent.owner, dependency.zone, Some(index)),
            )?;
        }
        self.pending = Some(pending);
        self.changes.sort_unstable();
        self.changes.dedup();
        for (active, edge) in self.changes.iter().copied() {
            if !self.graph.set(edge, active)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn accept(&mut self) {
        let pending = self.pending.take().expect("prepared Waiting admission");
        let zone = &mut self.zones[self.holds[pending.hold].zone];
        for index in &self.watchers[zone.watcher_cursor..pending.cursor] {
            self.dependencies[*index].blocked = true;
        }
        zone.count = pending.count;
        zone.used = pending.used;
        zone.watcher_cursor = pending.cursor;
        self.holds[pending.hold].active = true;
        self.graph.accept();
        self.changes.clear();
    }

    pub(crate) fn rollback(&mut self) {
        self.graph.rollback();
        self.pending = None;
        self.changes.clear();
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            graph,
            owner_index,
            zone_index,
            plan_index,
            zones,
            holds,
            dependencies,
            watchers,
            changes,
            pending: _,
        } = self;
        fn bytes<T>(values: &Vec<T>) -> u64 {
            (values.capacity() * std::mem::size_of::<T>()) as u64
        }
        graph.retained_logical_bytes()
            + bytes(owner_index)
            + bytes(zone_index)
            + bytes(plan_index)
            + bytes(zones)
            + bytes(holds)
            + bytes(dependencies)
            + bytes(watchers)
            + bytes(changes)
    }
}

impl TrafficWorld {
    pub(crate) fn prepare_waiting_dependencies(
        &mut self,
        include_candidates: bool,
    ) -> Result<(), StepError> {
        let mut ledger = std::mem::take(&mut self.waiting_dependencies);
        ledger.reset();
        let result = self.build_waiting_dependencies(&mut ledger, include_candidates);
        if result.is_err() {
            ledger.reset();
            // 部分构图失败可能只创建了 owner node，尚未形成完整 hold。
            ledger.owner_index.fill(OwnerIndex::default());
            ledger.zone_index.fill(None);
        }
        self.waiting_dependencies = ledger;
        result
    }

    fn build_waiting_dependencies(
        &self,
        ledger: &mut WaitingDependencies,
        include_candidates: bool,
    ) -> Result<(), StepError> {
        if self.waiting_member_rows.is_empty()
            && !(include_candidates
                && self
                    .conflict_candidates
                    .iter()
                    .any(|candidate| candidate.waiting_zone.is_some()))
        {
            return Ok(());
        }
        ledger.prepare_indices(
            self.next_state_by_vehicle.len(),
            self.waiting_zones.len(),
            self.waiting_plans.len(),
        )?;
        for member in &self.waiting_member_rows {
            let state = self
                .vehicle_state(member.vehicle)
                .ok_or(StepError::WaitingInvariantViolation)?;
            let compiled = self
                .compiled_route(state.route)
                .ok_or(StepError::WaitingInvariantViolation)?;
            let index = compiled
                .waiting
                .partition_point(|item| item.release_hop < member.release_hop);
            let occurrence = compiled
                .waiting
                .get(index)
                .ok_or(StepError::WaitingInvariantViolation)?;
            if occurrence.zone != member.zone || occurrence.release_hop != member.release_hop {
                return Err(StepError::WaitingInvariantViolation);
            }
            self.add_waiting_dependency_hold(ledger, member.vehicle, index, None)?;
        }
        if include_candidates {
            for candidate in self
                .conflict_candidates
                .iter()
                .filter(|candidate| candidate.waiting_zone.is_some())
            {
                let plan = self.waiting_plan_by_vehicle[candidate.vehicle.index() as usize]
                    .map(decode)
                    .ok_or(StepError::WaitingInvariantViolation)?;
                self.add_waiting_dependency_hold(
                    ledger,
                    candidate.vehicle,
                    self.waiting_plans[plan].occurrence_index as usize,
                    Some(plan),
                )?;
            }
        }
        ledger.finish_prepare()
    }

    fn add_waiting_dependency_hold(
        &self,
        ledger: &mut WaitingDependencies,
        owner: VehicleHandle,
        occurrence_index: usize,
        plan: Option<usize>,
    ) -> Result<(), StepError> {
        let state = self
            .vehicle_state(owner)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let compiled = self
            .compiled_route(state.route)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let occurrence = *compiled
            .waiting
            .get(occurrence_index)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let relations = self.revision.traffic().relations();
        let maximum = relations
            .waiting_zone(occurrence.zone)
            .ok_or(StepError::WaitingInvariantViolation)?
            .max_occupancy();
        let zone = ledger.zone(occurrence.zone, maximum)?;
        let profile = relations
            .vehicle_profile(state.profile)
            .ok_or(StepError::WaitingInvariantViolation)?;
        let hold = ledger.hold(
            owner,
            zone,
            state.length_mm,
            profile.min_gap_mm(),
            plan.is_none(),
            plan,
        )?;
        let dependencies = compiled
            .waiting
            .get(occurrence_index + 1..occurrence.dependency_end as usize)
            .ok_or(StepError::WaitingInvariantViolation)?;
        for dependency in dependencies {
            let maximum = relations
                .waiting_zone(dependency.zone)
                .ok_or(StepError::WaitingInvariantViolation)?
                .max_occupancy();
            let zone = ledger.zone(dependency.zone, maximum)?;
            ledger.dependency(hold, zone, dependency.storage_length_mm)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict::{WaitingDependencyNode, contains_multi_owner_waiting_cycle};

    fn owner(index: u32) -> VehicleHandle {
        VehicleHandle::new(index, 0)
    }

    /// 独立重建完整 prospective 图，以 SCC 规范检验增量边更新。
    fn oracle_cycle(ledger: &WaitingDependencies, prospective: Option<usize>) -> bool {
        let active = |index: usize| ledger.holds[index].active || Some(index) == prospective;
        let mut loads: Vec<_> = ledger
            .zones
            .iter()
            .map(|zone| (zone.count, zone.used))
            .collect();
        if let Some(index) = prospective {
            let hold = ledger.holds[index];
            let load = &mut loads[hold.zone];
            load.1 += u64::from(hold.length) + if load.0 == 0 { 0 } else { u64::from(hold.gap) };
            load.0 += 1;
        }
        let mut edges = Vec::new();
        for (index, hold) in ledger
            .holds
            .iter()
            .enumerate()
            .filter(|(index, _)| active(*index))
        {
            let from = WaitingDependencyNode::Owner(hold.owner.index());
            edges.push((
                WaitingDependencyNode::Zone(ledger.zones[hold.zone].ordinal),
                from,
            ));
            for dependency in ledger
                .dependencies
                .iter()
                .filter(|dependency| dependency.hold == index)
            {
                if ledger.holds.iter().enumerate().any(|(other, held)| {
                    active(other) && held.owner == hold.owner && held.zone == dependency.zone
                }) {
                    continue;
                }
                let (count, used) = loads[dependency.zone];
                let extra =
                    u64::from(hold.length) + if count == 0 { 0 } else { u64::from(hold.gap) };
                if count >= ledger.zones[dependency.zone].maximum
                    || used + extra > u64::from(dependency.storage)
                {
                    edges.push((
                        from,
                        WaitingDependencyNode::Zone(ledger.zones[dependency.zone].ordinal),
                    ));
                }
            }
        }
        contains_multi_owner_waiting_cycle(&edges)
    }

    #[test]
    fn prospective_occupancy_closes_old_dependency_and_rejection_rolls_back() {
        let mut ledger = WaitingDependencies::default();
        assert_eq!(ledger.retained_logical_bytes(), 0);
        ledger.prepare_indices(3, 2, 1).unwrap();
        let a = ledger.zone(WaitingZoneOrdinal::from_raw(0), 1).unwrap();
        let b = ledger.zone(WaitingZoneOrdinal::from_raw(1), 2).unwrap();
        let first = ledger.hold(owner(0), a, 10, 2, true, None).unwrap();
        ledger.dependency(first, b, 100).unwrap();
        ledger.hold(owner(2), b, 10, 2, true, None).unwrap();
        let candidate = ledger.hold(owner(1), b, 10, 2, false, Some(0)).unwrap();
        ledger.dependency(candidate, a, 100).unwrap();
        ledger.finish_prepare().unwrap();
        assert!(ledger.retained_logical_bytes() > 0);
        assert!(!oracle_cycle(&ledger, None));
        assert!(oracle_cycle(&ledger, Some(candidate)));
        for _ in 0..3 {
            assert!(!ledger.stage(0).unwrap());
            assert_eq!((ledger.zones[b].count, ledger.zones[b].used), (1, 10));
            assert!(!ledger.holds[candidate].active);
            assert!(!oracle_cycle(&ledger, None));
        }
    }

    #[test]
    fn old_and_prospective_holds_exclude_self_wait_and_keep_storage_threshold() {
        let mut ledger = WaitingDependencies::default();
        ledger.prepare_indices(1, 2, 1).unwrap();
        let a = ledger.zone(WaitingZoneOrdinal::from_raw(0), 1).unwrap();
        let b = ledger.zone(WaitingZoneOrdinal::from_raw(1), 2).unwrap();
        let old = ledger.hold(owner(0), a, 10, 90, true, None).unwrap();
        ledger.dependency(old, b, 10).unwrap();
        let candidate = ledger.hold(owner(0), b, 10, 90, false, Some(0)).unwrap();
        ledger.dependency(candidate, a, 10).unwrap();
        ledger.finish_prepare().unwrap();
        assert!(!oracle_cycle(&ledger, Some(candidate)));
        assert!(ledger.stage(0).unwrap());
        ledger.accept();
        assert_eq!(
            ledger.zones[b].used, 10,
            "empty zone does not charge the first member's gap"
        );
        assert!(!oracle_cycle(&ledger, None));
    }

    #[test]
    fn incremental_capacity_graph_matches_complete_scc_across_candidate_batches() {
        let mut seed = 71_u64;
        let mut random = |modulus: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 23) % modulus
        };
        let mut checked = 0;
        for _ in 0..200 {
            let mut ledger = WaitingDependencies::default();
            ledger.prepare_indices(12, 6, 8).unwrap();
            for zone in 0..6 {
                ledger.zone(WaitingZoneOrdinal::from_raw(zone), 4).unwrap();
            }
            for index in 0..12 {
                let zone = random(6) as usize;
                let hold = ledger
                    .hold(
                        owner(index),
                        zone,
                        10 + random(20) as u32,
                        random(20) as u32,
                        index < 4,
                        (index >= 4).then_some(index.saturating_sub(4) as usize),
                    )
                    .unwrap();
                for _ in 0..3 {
                    ledger
                        .dependency(hold, random(6) as usize, 30 + random(100) as u32)
                        .unwrap();
                }
            }
            if oracle_cycle(&ledger, None) {
                assert!(ledger.finish_prepare().is_err());
                continue;
            }
            ledger.finish_prepare().unwrap();
            for plan in 0..8 {
                let hold = decode(ledger.plan_index[plan].unwrap());
                let zone = ledger.zones[ledger.holds[hold].zone];
                if zone.count == zone.maximum {
                    continue;
                }
                let expected = !oracle_cycle(&ledger, Some(hold));
                assert_eq!(ledger.stage(plan).unwrap(), expected, "candidate {plan}");
                ledger.rollback();
                assert_eq!(ledger.stage(plan).unwrap(), expected, "retry {plan}");
                if expected {
                    ledger.accept();
                }
                assert!(!oracle_cycle(&ledger, None));
                checked += 1;
            }
        }
        assert!(checked > 500);
    }
}
