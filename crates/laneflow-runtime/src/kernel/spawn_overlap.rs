//! 道路准入的物理边候选索引；车身区间仍由共享展开函数定义。

use laneflow_static_contract::LaneEdgeOrdinal;

use crate::kernel::tables::{
    RouteSlot, VehicleSlot, admission_intervals_overlap, for_each_admission_interval,
};
use crate::{RouteHandle, TrafficWorld, VehicleHandle, VehicleState, VehicleStatus};

/// 可从活动车辆重建的缓存；不进入快照或摘要。
pub(crate) struct SpawnOverlapIndex {
    buckets: Vec<Vec<VehicleHandle>>,
    stale: bool,
}

impl Default for SpawnOverlapIndex {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            stale: true,
        }
    }
}

impl SpawnOverlapIndex {
    pub(crate) fn mark_stale(&mut self) {
        self.stale = true;
    }

    fn refresh(
        &mut self,
        lengths: &[u32],
        routes: &[RouteSlot],
        vehicles: &[VehicleSlot],
        active_order: &[VehicleHandle],
    ) {
        if !self.stale {
            return;
        }
        self.buckets.resize_with(lengths.len(), Vec::new);
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        self.stale = false;
        for &handle in active_order {
            let state = vehicles[handle.index() as usize]
                .state
                .expect("active order has a live vehicle");
            self.insert(lengths, routes, state);
        }
    }

    fn insert(&mut self, lengths: &[u32], routes: &[RouteSlot], state: VehicleState) {
        if self.stale || state.status != VehicleStatus::Active {
            return;
        }
        let edges = &routes[state.route.index() as usize]
            .compiled
            .as_ref()
            .expect("active vehicle has a compiled route")
            .edges;
        for_each_admission_interval(
            lengths,
            edges,
            state.route_edge_index as usize,
            state.progress_mm,
            state.length_mm,
            |edge, _, _| self.buckets[edge.index()].push(state.handle),
        )
        .expect("active vehicle has a valid footprint");
    }

    fn remove(&mut self, lengths: &[u32], routes: &[RouteSlot], state: VehicleState) {
        if self.stale || state.status != VehicleStatus::Active {
            return;
        }
        let edges = &routes[state.route.index() as usize]
            .compiled
            .as_ref()
            .expect("active vehicle has a compiled route")
            .edges;
        for_each_admission_interval(
            lengths,
            edges,
            state.route_edge_index as usize,
            state.progress_mm,
            state.length_mm,
            |edge, _, _| self.buckets[edge.index()].retain(|handle| *handle != state.handle),
        )
        .expect("active vehicle has a valid footprint");
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self { buckets, stale: _ } = self;
        crate::kernel::state::vec_bytes(buckets)
            + buckets
                .iter()
                .map(crate::kernel::state::vec_bytes)
                .sum::<u64>()
    }
}

impl TrafficWorld {
    /// 批量状态变动后首次查询重建；单次命令只登记/移除自己的实际占用边。
    pub(crate) fn overlap_blocker(
        &mut self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
    ) -> Option<VehicleHandle> {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        self.derived.spawn_overlap.refresh(
            lengths,
            &self.committed.routes,
            &self.committed.vehicles,
            &self.derived.active_order,
        );
        let spawn_edges = self.route_edges(route)?;
        let mut blocker: Option<VehicleHandle> = None;
        for_each_admission_interval(
            lengths,
            spawn_edges,
            cursor,
            progress,
            length,
            |edge, lo, hi| {
                for &handle in &self.derived.spawn_overlap.buckets[edge.index()] {
                    #[cfg(test)]
                    super::world::count_overlap_blocker_inspection();
                    // 多个 blocker 取槽位/世代最小值，不依赖桶的插入或重建顺序。
                    if blocker.is_some_and(|current| handle_key(current) <= handle_key(handle)) {
                        continue;
                    }
                    let state = self.vehicle_state(handle).expect("indexed live vehicle");
                    let edges = self.route_edges(state.route).expect("indexed route");
                    let mut overlaps = false;
                    for_each_admission_interval(
                        lengths,
                        edges,
                        state.route_edge_index as usize,
                        state.progress_mm,
                        state.length_mm,
                        |other: LaneEdgeOrdinal, other_lo, other_hi| {
                            overlaps |= edge == other
                                && admission_intervals_overlap(lo, hi, other_lo, other_hi);
                        },
                    )
                    .expect("indexed valid footprint");
                    if overlaps {
                        blocker = Some(handle);
                    }
                }
            },
        )?;
        blocker
    }

    pub(crate) fn register_overlap_vehicle(&mut self, state: VehicleState) {
        self.derived.spawn_overlap.insert(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &self.committed.routes,
            state,
        );
    }

    /// 在清除/改变旧状态及其路线引用之前调用。
    pub(crate) fn unregister_overlap_vehicle(&mut self, state: VehicleState) {
        self.derived.spawn_overlap.remove(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &self.committed.routes,
            state,
        );
    }
}

fn handle_key(handle: VehicleHandle) -> (u32, u32) {
    (handle.index(), handle.generation())
}
