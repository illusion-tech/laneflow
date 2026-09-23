//! 停车成员变更的保序维护；定位只缓存 live 顺序，不拥有新的车辆顺序权威。

use crate::VehicleHandle;

#[cfg(test)]
#[path = "tests/active_order.rs"]
mod tests;

/// 已登记 live 前缀的 slot -> 顺序位置；查询再以完整句柄核对，空闲槽值不可复用。
/// append 保留前缀，删除/替换/重排必须 invalidate；不随状态序号或普通 step 失效。
#[derive(Default)]
pub(crate) struct LiveOrderIndex {
    positions: Vec<u32>,
    indexed_len: usize,
}

impl LiveOrderIndex {
    pub(crate) fn invalidate(&mut self) {
        self.indexed_len = 0;
    }

    /// 序号表已经盖住当前 live 前缀时才可以直接查，避免为一次删除把冷缓存铺开。
    pub(crate) fn is_current(&self, live_len: usize) -> bool {
        self.indexed_len == live_len && live_len > 0
    }

    /// 同一 live 序号换成另一个槽位。长度不变，后面的序号不用重铺。
    pub(crate) fn retarget(&mut self, vehicle: VehicleHandle, rank: usize) {
        if self.indexed_len == 0 {
            return;
        }
        let Ok(rank) = u32::try_from(rank) else {
            self.invalidate();
            return;
        };
        let slot = vehicle.index() as usize;
        if slot >= self.positions.len() {
            self.invalidate();
            return;
        }
        self.positions[slot] = rank;
    }

    fn prepare(&mut self, live: &[VehicleHandle], slots: usize) -> bool {
        if self.indexed_len == live.len() {
            return true;
        }
        assert!(
            self.indexed_len <= live.len(),
            "live prefix must be invalidated"
        );
        if slots > self.positions.capacity() {
            #[cfg(test)]
            super::parking_command_research::note(|counts| counts.position_reserves += 1);
            if !reserve_positions(&mut self.positions, slots) {
                #[cfg(test)]
                super::parking_command_research::note(|counts| counts.position_failures += 1);
                return false;
            }
        }
        if slots > self.positions.len() {
            self.positions.resize(slots, u32::MAX);
        }
        #[cfg(test)]
        super::parking_command_research::note(|counts| {
            counts.position_builds += 1;
            counts.position_visits += live.len() - self.indexed_len;
        });
        for (rank, handle) in live.iter().enumerate().skip(self.indexed_len) {
            self.positions[handle.index() as usize] =
                u32::try_from(rank).expect("live rank fits vehicle capacity");
        }
        self.indexed_len = live.len();
        true
    }

    fn position(&self, vehicle: VehicleHandle, live: &[VehicleHandle]) -> Option<usize> {
        let rank = *self.positions.get(vehicle.index() as usize)? as usize;
        (rank < self.indexed_len && live.get(rank) == Some(&vehicle)).then_some(rank)
    }

    /// 准备一次后按槽位读取 live 序号。句柄与该序号上的 live 项不一致时返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 序号表分配失败时返回 `Err(())`。调用方不得改用槽位下标充当 live 序号。
    pub(crate) fn rank(
        &mut self,
        live: &[VehicleHandle],
        slots: usize,
        vehicle: VehicleHandle,
    ) -> Result<Option<u32>, ()> {
        if !self.prepare(live, slots) {
            return Err(());
        }
        Ok(self
            .position(vehicle, live)
            .map(|position| u32::try_from(position).expect("live rank fits vehicle capacity")))
    }

    #[cfg(test)]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        super::state::vec_bytes(&self.positions)
    }
}

fn reserve_positions(positions: &mut Vec<u32>, slots: usize) -> bool {
    #[cfg(test)]
    if FAIL_RESERVATION.with(core::cell::Cell::get) {
        return false;
    }
    positions.try_reserve_exact(slots - positions.len()).is_ok()
}

#[cfg(test)]
thread_local! {
    static FAIL_RESERVATION: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn with_position_allocation_failure<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL_RESERVATION.with(|value| value.set(self.0));
        }
    }
    FAIL_RESERVATION.with(|value| {
        let _reset = Reset(value.replace(true));
        run()
    })
}

impl crate::kernel::state::WorldState {
    /// 全部领域检查之后、提交之前准备可选定位；失败只选择既有全量投影。
    pub(crate) fn prepare_active_insertion(&mut self, vehicle: VehicleHandle) -> Option<usize> {
        let live = &self.committed.live_order;
        let positions = &mut self.derived.live_order_index;
        if !positions.prepare(live, self.committed.vehicles.len()) {
            return None;
        }
        let rank = positions
            .position(vehicle, live)
            .expect("validated live vehicle");
        Some(self.derived.active_order.partition_point(|active| {
            positions
                .position(*active, live)
                .expect("active is a live projection")
                < rank
        }))
    }

    /// Active Vec 已在安装/迁移时预留完整车辆容量；提交段不能新增可失败预留。
    pub(crate) fn insert_active_vehicle(&mut self, vehicle: VehicleHandle, index: Option<usize>) {
        if let Some(index) = index {
            assert!(self.derived.active_order.len() < self.derived.active_order.capacity());
            #[cfg(test)]
            super::parking_command_research::note(|counts| {
                counts.active_insertions += 1;
                counts.active_writes += self.derived.active_order.len() - index + 1;
            });
            self.derived.active_order.insert(index, vehicle);
        } else {
            self.rebuild_active_order();
        }
    }

    /// 已完成的车若还占着原来的在驶名次，换成新句柄。步进提交已经移出时返回 `false`。
    pub(crate) fn replace_active_handle(&mut self, old: VehicleHandle, new: VehicleHandle) -> bool {
        let Some(index) = self
            .derived
            .active_order
            .iter()
            .position(|active| *active == old)
        else {
            return false;
        };
        self.derived.active_order[index] = new;
        true
    }

    pub(crate) fn remove_active_vehicle(&mut self, vehicle: VehicleHandle) {
        let index = self
            .derived
            .active_order
            .iter()
            .position(|active| *active == vehicle)
            .expect("parked vehicle was active before commit");
        #[cfg(test)]
        super::parking_command_research::note(|counts| {
            counts.active_removals += 1;
            counts.active_writes += self.derived.active_order.len() - index - 1;
        });
        self.derived.active_order.remove(index);
    }
}

#[cfg(test)]
use crate::TrafficWorld;
