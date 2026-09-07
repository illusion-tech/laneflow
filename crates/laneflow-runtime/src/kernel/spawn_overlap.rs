//! 道路准入的物理边候选索引；车身区间仍由共享展开函数定义。

use laneflow_static_contract::LaneEdgeOrdinal;

use crate::kernel::tables::{
    RouteSlot, VehicleSlot, admission_intervals_overlap, for_each_admission_interval,
};
use crate::{RouteHandle, TrafficWorld, VehicleHandle, VehicleState, VehicleStatus};

#[cfg(test)]
thread_local! {
    static RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
    static REBUILDS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// 测试用：令索引在第 `count` 次成功预留之后注入分配失败。
#[cfg(test)]
pub(crate) fn with_overlap_allocation_failure_after<T>(count: usize, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<usize>);
    impl Drop for Reset {
        fn drop(&mut self) {
            RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
        }
    }
    RESERVATIONS_BEFORE_FAILURE.with(|remaining| {
        let _reset = Reset(remaining.replace(Some(count)));
        run()
    })
}

/// 测试用：返回索引重建次数。
#[cfg(test)]
pub(crate) fn overlap_rebuilds() -> usize {
    REBUILDS.with(core::cell::Cell::get)
}

fn try_reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), ()> {
    if values.capacity() - values.len() >= additional {
        return Ok(());
    }
    #[cfg(test)]
    if RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(value) => {
            remaining.set(Some(value - 1));
            false
        }
        None => false,
    }) {
        return Err(());
    }
    values.try_reserve(additional).map_err(|_| ())
}

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
    /// 索引是否与当前已提交状态一致。
    pub(crate) fn is_current(&self) -> bool {
        !self.stale
    }

    /// 标记索引失效；下次查询前重建。
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

    /// 切换候选的可失败重建。未通过实体校验的坏 footprint 不参与候选过滤，
    /// 仍由原 live 序中的实体校验报告错误，不能让索引提前改变首错。
    fn try_refresh(
        &mut self,
        lengths: &[u32],
        routes: &[RouteSlot],
        vehicles: &[VehicleSlot],
        active_order: &[VehicleHandle],
    ) -> Result<(), ()> {
        if !self.stale {
            return Ok(());
        }
        let additional = lengths.len().saturating_sub(self.buckets.len());
        try_reserve(&mut self.buckets, additional)?;
        self.buckets.resize_with(lengths.len(), Vec::new);
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        for &handle in active_order {
            if let Some(state) = vehicles
                .get(handle.index() as usize)
                .and_then(|slot| slot.state)
                .filter(|state| state.handle == handle)
            {
                self.try_insert_footprint(lengths, routes, state)?;
            }
        }
        self.stale = false;
        #[cfg(test)]
        REBUILDS.with(|count| count.set(count.get() + 1));
        Ok(())
    }

    fn try_insert_footprint(
        &mut self,
        lengths: &[u32],
        routes: &[RouteSlot],
        state: VehicleState,
    ) -> Result<(), ()> {
        if state.status != VehicleStatus::Active {
            return Ok(());
        }
        let Some(compiled) = routes
            .get(state.route.index() as usize)
            .filter(|slot| slot.generation == state.route.generation())
            .and_then(|slot| slot.compiled.as_ref())
        else {
            return Ok(());
        };
        if for_each_admission_interval(
            lengths,
            &compiled.edges,
            state.route_edge_index as usize,
            state.progress_mm,
            state.length_mm,
            |_, _, _| {},
        )
        .is_none()
        {
            return Ok(());
        }
        let mut result = Ok(());
        for_each_admission_interval(
            lengths,
            &compiled.edges,
            state.route_edge_index as usize,
            state.progress_mm,
            state.length_mm,
            |edge, _, _| {
                if result.is_ok() {
                    let bucket = &mut self.buckets[edge.index()];
                    result = try_reserve(bucket, 1);
                    if result.is_ok() {
                        bucket.push(state.handle);
                    }
                }
            },
        );
        if result.is_err() {
            self.stale = true;
        }
        result
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

    /// 测试用：索引持有的逻辑字节数。
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
        self.indexed_overlap_blocker(route, cursor, progress, length, None)
    }

    /// 索引失效时可失败重建；失败时保持失效，首错仍由实体校验报告。
    pub(crate) fn try_refresh_overlap_index(&mut self) -> Result<(), ()> {
        self.derived.spawn_overlap.try_refresh(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &self.committed.routes,
            &self.committed.vehicles,
            &self.derived.active_order,
        )
    }

    /// 在已刷新索引上查询候选准入 footprint 的阻挡车辆；多个阻挡取槽位/世代最小者。
    pub(crate) fn indexed_overlap_blocker(
        &self,
        route: RouteHandle,
        cursor: usize,
        progress: u32,
        length: u32,
        excluded: Option<VehicleHandle>,
    ) -> Option<VehicleHandle> {
        debug_assert!(self.derived.spawn_overlap.is_current());
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
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
                    if Some(handle) == excluded {
                        continue;
                    }
                    #[cfg(test)]
                    super::world::count_overlap_blocker_inspection();
                    // 多个 blocker 取槽位/世代最小值，不依赖桶的插入或重建顺序。
                    if blocker.is_some_and(|current| handle_key(current) <= handle_key(handle)) {
                        continue;
                    }
                    let Some(state) = self.vehicle_state(handle) else {
                        continue;
                    };
                    let Some(edges) = self.route_edges(state.route) else {
                        continue;
                    };
                    let mut overlaps = false;
                    let complete = for_each_admission_interval(
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
                    .is_some();
                    if complete && overlaps {
                        blocker = Some(handle);
                    }
                }
            },
        )?;
        blocker
    }

    /// 把单车的实际占用边登记进当前索引。
    pub(crate) fn register_overlap_vehicle(&mut self, state: VehicleState) {
        self.derived.spawn_overlap.insert(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &self.committed.routes,
            state,
        );
    }

    /// 重放前索引为 current 时，在单车重验证通过后补登记；冷重建已包含新车辆。
    pub(crate) fn try_register_overlap_vehicle(&mut self, state: VehicleState) -> Result<(), ()> {
        if !self.derived.spawn_overlap.is_current() {
            return Ok(());
        }
        self.derived.spawn_overlap.try_insert_footprint(
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &self.committed.routes,
            state,
        )
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
