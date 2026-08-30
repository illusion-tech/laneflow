//! 迁移增量日志（#302 切换合同 §5；#513 切片 C-1）：已提交变更流的有界物化。
//!
//! 日志在切换事务 Prepare 边界原子武装：arena 按字节上界一次预留，此后
//! 旧世界每次提交把已提交变更写进预留空间——武装期稳态 tick 不因准备新增
//! 分配（切换合同 §9 在线准备干扰口径）。溢出是粘性失败：置位后丢弃后续
//! 记录，旧世界步进不受影响；事务在下个边界观察到溢出即放弃候选，整个
//! 日志随之废弃（不截断、不丢弃尾记录后续写、不降级为全量重放）。
//!
//! 编码是本进程内的小端字节流，不是持久制品。记录用槽位下标 + generation
//! （即句柄）作耐久键：日志只在同一进程、同一槽位布局的旧世界与候选之间
//! 搬运（候选由 Prepare 时的结构克隆构造），不存在跨进程重解释；generation
//! 消解窗口期内槽位回收的歧义。覆盖区间为半开区间 [基线游标, 静默点提交
//! 游标)：命令记录携带自身提交游标，静默边界上最终游标由事务在同一原子
//! 边界取样，不存在既不在基线也不在日志、也不由最终游标定归属的提交。

use laneflow_static_contract::LaneEdgeOrdinal;
use thiserror::Error;

use crate::{RouteHandle, VehicleHandle, VehicleState, VehicleStatus};

/// 迁移增量日志字节上界的文档化默认值（切换合同 §5：默认值由容量合同登记）。
///
/// v1 取 8 MiB：按每 tick 每活跃车辆 48 字节条目估算，千车量级世界约可覆盖
/// 一百余 tick 的在线追赶窗口；更大的世界按比例缩小可追窗口，超界由事务
/// 失败关闭放弃、宿主显式改用维护暂停模式重试。初值随切片 C 证据登记
/// （合同 §9「迁移增量日志」行）。
pub const DEFAULT_MIGRATION_DELTA_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;

/// 武装中迁移增量日志的宿主可观测统计快照（#513 切片 C：追赶滞后与
/// 后台资源预算的观测面）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationJournalStats {
    /// 字节上界（arena 预留量）。
    pub byte_bound: u64,
    /// 已写入字节数。
    pub written_bytes: u64,
    /// 成功写入的记录数。
    pub record_count: u64,
    /// 是否已溢出（粘性）。
    pub overflowed: bool,
    /// 武装后首条 TICK 记录的 tick 序号。
    pub first_tick: Option<u64>,
    /// 最近一条 TICK 记录的 tick 序号。
    pub last_tick: Option<u64>,
    /// 覆盖区间下界的基线命令游标。
    pub baseline_command_cursor: u64,
}

/// 日志武装失败（切换合同 §8 按「候选构造失败」处置：丢弃候选，旧世界无感知）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(crate) enum MigrationJournalError {
    /// arena 预留失败（含上界超出本机 `usize` 可寻址范围）。
    #[error("迁移增量日志 arena 预留失败（{requested} 字节）")]
    ArenaReserveFailed {
        /// 请求预留的字节数。
        requested: u64,
    },
    /// 已存在武装中的日志（在途唯一：每世界至多一个在途切换事务）。
    #[error("迁移增量日志已在武装中")]
    AlreadyArmed,
}

// 记录种类标签。封闭集合；未知标签是编码器/解码器漂移，按内部不变量处置。
const TAG_TICK: u8 = 1;
const TAG_ROUTE_REGISTERED: u8 = 2;
const TAG_ROUTE_REMOVED: u8 = 3;
const TAG_VEHICLE_SPAWNED: u8 = 4;
const TAG_VEHICLE_REPLACED: u8 = 5;
const TAG_PARKING_OCCUPIED: u8 = 6;

// 状态封闭 u8 编码（与快照摘要 §6 的车辆记录一致）。
const STATUS_ACTIVE: u8 = 1;
const STATUS_PARKED: u8 = 2;
const STATUS_COMPLETED: u8 = 3;

// TICK 记录头：tag + tick_index + time_ms + entry_count。
const TICK_HEADER_BYTES: usize = 1 + 8 + 8 + 4;

/// 单条车辆增量：tick 条目与生成/替换记录共用，固定 48 字节小端布局。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VehicleDelta {
    /// 车辆槽位下标（句柄 index）。
    pub(crate) slot: u32,
    /// 车辆槽位 generation（句柄 generation）。
    pub(crate) generation: u32,
    /// profile 序数（base 侧原始值；消费方经 LFSD 映射重绑）。
    pub(crate) profile: u32,
    /// 参与者类别序数（base 侧原始值）。
    pub(crate) class: u32,
    /// 绑定路线槽位下标。
    pub(crate) route_index: u32,
    /// 绑定路线 generation。
    pub(crate) route_generation: u32,
    /// 路线序列下标。
    pub(crate) route_edge_index: u32,
    /// 当前边进度（毫米）。
    pub(crate) progress_mm: u32,
    /// 未凑满 1 mm 的微米余数。
    pub(crate) carry_um: u16,
    /// 已提交速度（毫米每秒）。
    pub(crate) speed_mm_s: u32,
    /// 车身长度（毫米）。
    pub(crate) length_mm: u32,
    /// 生命周期状态。
    pub(crate) status: VehicleStatus,
    /// 停车位序数原始值（base 侧；`None` = 未占车位）。
    pub(crate) parking: Option<u32>,
}

impl VehicleDelta {
    /// 从已提交车辆状态提取增量。
    pub(crate) fn from_state(state: &VehicleState) -> Self {
        Self {
            slot: state.handle().index(),
            generation: state.handle().generation(),
            profile: state.profile().raw(),
            class: state.class().raw(),
            route_index: state.route().index(),
            route_generation: state.route().generation(),
            route_edge_index: state.route_edge_index(),
            progress_mm: state.progress_mm(),
            carry_um: state.carry_um(),
            speed_mm_s: state.speed_mm_s(),
            length_mm: state.length_mm(),
            status: state.status(),
            parking: state.parking().map(|space| space.raw()),
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        put_u32(out, self.slot);
        put_u32(out, self.generation);
        put_u32(out, self.profile);
        put_u32(out, self.class);
        put_u32(out, self.route_index);
        put_u32(out, self.route_generation);
        put_u32(out, self.route_edge_index);
        put_u32(out, self.progress_mm);
        put_u16(out, self.carry_um);
        put_u32(out, self.speed_mm_s);
        put_u32(out, self.length_mm);
        put_u8(out, status_to_raw(self.status));
        put_u8(out, u8::from(self.parking.is_some()));
        put_u32(out, self.parking.unwrap_or(0));
    }

    pub(crate) fn decode(bytes: &[u8]) -> Self {
        assert!(
            bytes.len() >= VEHICLE_DELTA_BYTES,
            "vehicle delta needs full width"
        );
        let parking_present = bytes[43] == 1;
        Self {
            slot: read_u32(bytes, 0),
            generation: read_u32(bytes, 4),
            profile: read_u32(bytes, 8),
            class: read_u32(bytes, 12),
            route_index: read_u32(bytes, 16),
            route_generation: read_u32(bytes, 20),
            route_edge_index: read_u32(bytes, 24),
            progress_mm: read_u32(bytes, 28),
            carry_um: read_u16(bytes, 32),
            speed_mm_s: read_u32(bytes, 34),
            length_mm: read_u32(bytes, 38),
            status: status_from_raw(bytes[42]),
            parking: parking_present.then(|| read_u32(bytes, 44)),
        }
    }
}

/// 车辆增量固定字节宽度。
pub(crate) const VEHICLE_DELTA_BYTES: usize = 48;

fn status_to_raw(status: VehicleStatus) -> u8 {
    match status {
        VehicleStatus::Active => STATUS_ACTIVE,
        VehicleStatus::Parked => STATUS_PARKED,
        VehicleStatus::Completed => STATUS_COMPLETED,
    }
}

fn status_from_raw(raw: u8) -> VehicleStatus {
    match raw {
        STATUS_ACTIVE => VehicleStatus::Active,
        STATUS_PARKED => VehicleStatus::Parked,
        STATUS_COMPLETED => VehicleStatus::Completed,
        other => panic!("unknown vehicle status raw {other}"),
    }
}

/// 解码后的单条日志记录。边序列与 tick 条目流以原始小端 `u32` 字节段借出，
/// 由消费方（候选追赶）按需迭代。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalRecord<'a> {
    /// 一个固定步进边界的已提交动态增量。每个成功 `step` 恰一条，即使零条目
    /// （tick/时间是候选侧时钟与摘要头部的收敛依据，不可省略）。
    Tick {
        /// 已提交 tick 序号。
        tick_index: u64,
        /// 已提交世界时间（毫秒）。
        time_ms: u64,
        /// 48 字节步长的 [`VehicleDelta`] 流。
        entries: &'a [u8],
    },
    /// 路线注册（base 侧边序数；消费方经 LFSD 重绑到 target 再编译验证）。
    RouteRegistered {
        /// 提交时的命令游标。
        command_cursor: u64,
        /// 路线槽位下标。
        slot: u32,
        /// 路线槽位 generation。
        generation: u32,
        /// 4 字节步长的边序数原始值流。
        edges: &'a [u8],
    },
    /// 路线移除（仅无引用路线可成功）。
    RouteRemoved {
        /// 提交时的命令游标。
        command_cursor: u64,
        /// 路线槽位下标。
        slot: u32,
        /// 槽位 generation 是否可继续递增并回收。
        recyclable: bool,
        /// 移除后的槽位 generation。
        generation_after: u32,
    },
    /// 车辆生成。
    VehicleSpawned {
        /// 提交时的命令游标。
        command_cursor: u64,
        /// 新车辆增量（含句柄槽位与全部整值状态）。
        vehicle: VehicleDelta,
    },
    /// Completed 车辆原子替换：旧句柄立即 stale，新句柄占用 live 序原位。
    VehicleReplaced {
        /// 提交时的命令游标。
        command_cursor: u64,
        /// 失效旧句柄的槽位下标。
        old_slot: u32,
        /// 失效旧句柄的 generation。
        old_generation: u32,
        /// 替换在 live 序中的原位下标。
        order_index: u32,
        /// 新车辆增量。
        vehicle: VehicleDelta,
    },
    /// 停车占用（只记有状态变化的成功占用；幂等重占不产生动态增量，不记）。
    ParkingOccupied {
        /// 提交时的命令游标。
        command_cursor: u64,
        /// 车辆槽位下标。
        slot: u32,
        /// 车辆槽位 generation。
        generation: u32,
        /// 车位序数原始值（base 侧）。
        space: u32,
    },
}

/// 迁移增量日志：已预留 arena + 粘性溢出标记与只读统计。
pub(crate) struct MigrationDeltaJournal {
    bytes: Vec<u8>,
    byte_bound: u64,
    overflowed: bool,
    record_count: u64,
    first_tick: Option<u64>,
    last_tick: Option<u64>,
    baseline_command_cursor: u64,
    /// 打开的 TICK 记录的 entry_count 字段在 arena 中的绝对偏移。
    open_tick_count_at: Option<usize>,
    /// 当前打开 TICK 记录已成功写入的条目数。
    open_tick_entries: u32,
}

impl MigrationDeltaJournal {
    /// 武装日志：按字节上界一次预留 arena。基线命令游标只作覆盖区间下界的
    /// 登记与断言素材，不参与写入路径。
    pub(crate) fn arm(
        byte_bound: u64,
        baseline_command_cursor: u64,
    ) -> Result<Self, MigrationJournalError> {
        let capacity =
            usize::try_from(byte_bound).map_err(|_| MigrationJournalError::ArenaReserveFailed {
                requested: byte_bound,
            })?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| {
            MigrationJournalError::ArenaReserveFailed {
                requested: byte_bound,
            }
        })?;
        Ok(Self {
            bytes,
            byte_bound,
            overflowed: false,
            record_count: 0,
            first_tick: None,
            last_tick: None,
            baseline_command_cursor,
            open_tick_count_at: None,
            open_tick_entries: 0,
        })
    }

    /// 是否已溢出（粘性）。
    pub(crate) const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// 已写入字节数。
    pub(crate) fn written_bytes(&self) -> u64 {
        u64::try_from(self.bytes.len()).expect("journal length fits u64")
    }

    /// 成功写入的记录数。
    pub(crate) const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// 武装后首条 TICK 记录的 tick 序号。
    pub(crate) const fn first_tick(&self) -> Option<u64> {
        self.first_tick
    }

    /// 最近一条 TICK 记录的 tick 序号。
    pub(crate) const fn last_tick(&self) -> Option<u64> {
        self.last_tick
    }

    /// 覆盖区间下界的基线命令游标。
    pub(crate) const fn baseline_command_cursor(&self) -> u64 {
        self.baseline_command_cursor
    }

    /// 宿主可观测统计快照：滞后、字节占用、溢出与覆盖区间下界。
    #[must_use]
    pub fn stats(&self) -> MigrationJournalStats {
        MigrationJournalStats {
            byte_bound: self.byte_bound,
            written_bytes: self.written_bytes(),
            record_count: self.record_count(),
            overflowed: self.overflowed(),
            first_tick: self.first_tick(),
            last_tick: self.last_tick(),
            baseline_command_cursor: self.baseline_command_cursor(),
        }
    }

    /// 从记录边界的字节偏移处续读（消费侧持久化游标；偏移必须由
    /// [`RecordIter::offset`] 产生且不超过已写字节）。
    pub(crate) fn records_from(&self, offset: usize) -> RecordIter<'_> {
        assert!(
            !self.overflowed,
            "overflowed migration journal must be abandoned, not decoded"
        );
        assert!(
            offset <= self.bytes.len(),
            "resumed offset beyond written journal bytes"
        );
        RecordIter {
            bytes: self.bytes.as_slice(),
            pos: offset,
        }
    }

    fn remaining(&self) -> usize {
        usize::try_from(self.byte_bound).expect("byte bound fits usize") - self.bytes.len()
    }

    /// 预检并粘性置位溢出。返回 `false` 时调用方必须丢弃本条记录。
    fn ensure(&mut self, needed: usize) -> bool {
        if self.overflowed {
            return false;
        }
        if self.remaining() < needed {
            self.overflowed = true;
            return false;
        }
        true
    }

    /// 打开一条 TICK 记录：写头（条目数占位为零），条目随后流式写入，
    /// [`Self::finish_tick`] 回填计数。零变化步进也必须开记录——tick/时间
    /// 是候选时钟与摘要头部的收敛依据。
    pub(crate) fn begin_tick(&mut self, tick_index: u64, time_ms: u64) {
        debug_assert!(
            self.open_tick_count_at.is_none(),
            "previous tick not closed"
        );
        if !self.ensure(TICK_HEADER_BYTES) {
            return;
        }
        let count_at = self.bytes.len() + 1 + 8 + 8;
        put_u8(&mut self.bytes, TAG_TICK);
        put_u64(&mut self.bytes, tick_index);
        put_u64(&mut self.bytes, time_ms);
        put_u32(&mut self.bytes, 0);
        self.open_tick_count_at = Some(count_at);
        self.open_tick_entries = 0;
        if self.first_tick.is_none() {
            self.first_tick = Some(tick_index);
        }
        self.last_tick = Some(tick_index);
    }

    /// 向打开的 TICK 记录写入一条车辆增量（无变化条目由调用方过滤）。
    pub(crate) fn tick_entry(&mut self, delta: &VehicleDelta) {
        let Some(_) = self.open_tick_count_at else {
            return;
        };
        if !self.ensure(VEHICLE_DELTA_BYTES) {
            return;
        }
        delta.encode(&mut self.bytes);
        self.open_tick_entries = self.open_tick_entries.saturating_add(1);
    }

    /// 关闭 TICK 记录并回填条目数。
    pub(crate) fn finish_tick(&mut self) {
        let Some(count_at) = self.open_tick_count_at.take() else {
            return;
        };
        let entries = self.open_tick_entries;
        write_u32_at(&mut self.bytes, count_at, entries);
        self.record_count += 1;
    }

    /// 路线注册记录。
    pub(crate) fn record_route_registered(
        &mut self,
        command_cursor: u64,
        route: RouteHandle,
        edges: &[LaneEdgeOrdinal],
    ) {
        let edge_count =
            u32::try_from(edges.len()).expect("route edge count fits journal u32 count");
        let needed = 1 + 8 + 4 + 4 + 4 + 4 * edges.len();
        if !self.ensure(needed) {
            return;
        }
        put_u8(&mut self.bytes, TAG_ROUTE_REGISTERED);
        put_u64(&mut self.bytes, command_cursor);
        put_u32(&mut self.bytes, route.index());
        put_u32(&mut self.bytes, route.generation());
        put_u32(&mut self.bytes, edge_count);
        for edge in edges {
            put_u32(&mut self.bytes, edge.raw());
        }
        self.record_count += 1;
    }

    /// 路线移除记录。
    pub(crate) fn record_route_removed(
        &mut self,
        command_cursor: u64,
        slot: u32,
        recyclable: bool,
        generation_after: u32,
    ) {
        if !self.ensure(1 + 8 + 4 + 1 + 4) {
            return;
        }
        put_u8(&mut self.bytes, TAG_ROUTE_REMOVED);
        put_u64(&mut self.bytes, command_cursor);
        put_u32(&mut self.bytes, slot);
        put_u8(&mut self.bytes, u8::from(recyclable));
        put_u32(&mut self.bytes, generation_after);
        self.record_count += 1;
    }

    /// 车辆生成记录。
    pub(crate) fn record_vehicle_spawned(&mut self, command_cursor: u64, vehicle: VehicleDelta) {
        if !self.ensure(1 + 8 + VEHICLE_DELTA_BYTES) {
            return;
        }
        put_u8(&mut self.bytes, TAG_VEHICLE_SPAWNED);
        put_u64(&mut self.bytes, command_cursor);
        vehicle.encode(&mut self.bytes);
        self.record_count += 1;
    }

    /// 车辆替换记录。
    pub(crate) fn record_vehicle_replaced(
        &mut self,
        command_cursor: u64,
        old: VehicleHandle,
        order_index: u32,
        vehicle: VehicleDelta,
    ) {
        if !self.ensure(1 + 8 + 4 + 4 + 4 + VEHICLE_DELTA_BYTES) {
            return;
        }
        put_u8(&mut self.bytes, TAG_VEHICLE_REPLACED);
        put_u64(&mut self.bytes, command_cursor);
        put_u32(&mut self.bytes, old.index());
        put_u32(&mut self.bytes, old.generation());
        put_u32(&mut self.bytes, order_index);
        vehicle.encode(&mut self.bytes);
        self.record_count += 1;
    }

    /// 停车占用记录。
    pub(crate) fn record_parking_occupied(
        &mut self,
        command_cursor: u64,
        vehicle: VehicleHandle,
        space: u32,
    ) {
        if !self.ensure(1 + 8 + 4 + 4 + 4) {
            return;
        }
        put_u8(&mut self.bytes, TAG_PARKING_OCCUPIED);
        put_u64(&mut self.bytes, command_cursor);
        put_u32(&mut self.bytes, vehicle.index());
        put_u32(&mut self.bytes, vehicle.generation());
        put_u32(&mut self.bytes, space);
        self.record_count += 1;
    }
}

/// 4 字节步长的小端 `u32` 流迭代（边序数 / tick 条目计数等原始段）。
pub(crate) fn raw_u32_stream(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| read_u32_chunk(chunk))
}

fn read_u32_chunk(chunk: &[u8]) -> u32 {
    u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
}

/// 日志解码迭代器。编码器与解码器同源演进；格式漂移按内部不变量 panic。
pub(crate) struct RecordIter<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> RecordIter<'a> {
    /// 当前消费位置（记录边界字节偏移；空日志或耗尽时为写入字节数）。
    pub(crate) const fn offset(&self) -> usize {
        self.pos
    }
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = JournalRecord<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let rest = &self.bytes[self.pos..];
        let tag = rest[0];
        let mut at = 1;
        let record = match tag {
            TAG_TICK => {
                let tick_index = read_u64(rest, at);
                at += 8;
                let time_ms = read_u64(rest, at);
                at += 8;
                let count = read_u32(rest, at) as usize;
                at += 4;
                let entries_len = count * VEHICLE_DELTA_BYTES;
                let entries = rest
                    .get(at..at + entries_len)
                    .expect("tick entries within journal");
                at += entries_len;
                JournalRecord::Tick {
                    tick_index,
                    time_ms,
                    entries,
                }
            }
            TAG_ROUTE_REGISTERED => {
                let command_cursor = read_u64(rest, at);
                at += 8;
                let slot = read_u32(rest, at);
                at += 4;
                let generation = read_u32(rest, at);
                at += 4;
                let count = read_u32(rest, at) as usize;
                at += 4;
                let edges_len = count * 4;
                let edges = rest.get(at..at + edges_len).expect("edges within journal");
                at += edges_len;
                JournalRecord::RouteRegistered {
                    command_cursor,
                    slot,
                    generation,
                    edges,
                }
            }
            TAG_ROUTE_REMOVED => {
                let command_cursor = read_u64(rest, at);
                at += 8;
                let slot = read_u32(rest, at);
                at += 4;
                let recyclable = rest[at] == 1;
                at += 1;
                let generation_after = read_u32(rest, at);
                at += 4;
                JournalRecord::RouteRemoved {
                    command_cursor,
                    slot,
                    recyclable,
                    generation_after,
                }
            }
            TAG_VEHICLE_SPAWNED => {
                let command_cursor = read_u64(rest, at);
                at += 8;
                let vehicle = VehicleDelta::decode(rest.get(at..).expect("spawn delta present"));
                at += VEHICLE_DELTA_BYTES;
                JournalRecord::VehicleSpawned {
                    command_cursor,
                    vehicle,
                }
            }
            TAG_VEHICLE_REPLACED => {
                let command_cursor = read_u64(rest, at);
                at += 8;
                let old_slot = read_u32(rest, at);
                at += 4;
                let old_generation = read_u32(rest, at);
                at += 4;
                let order_index = read_u32(rest, at);
                at += 4;
                let vehicle = VehicleDelta::decode(rest.get(at..).expect("replace delta present"));
                at += VEHICLE_DELTA_BYTES;
                JournalRecord::VehicleReplaced {
                    command_cursor,
                    old_slot,
                    old_generation,
                    order_index,
                    vehicle,
                }
            }
            TAG_PARKING_OCCUPIED => {
                let command_cursor = read_u64(rest, at);
                at += 8;
                let slot = read_u32(rest, at);
                at += 4;
                let generation = read_u32(rest, at);
                at += 4;
                let space = read_u32(rest, at);
                at += 4;
                JournalRecord::ParkingOccupied {
                    command_cursor,
                    slot,
                    generation,
                    space,
                }
            }
            other => panic!("unknown migration journal tag {other}"),
        };
        self.pos += at;
        Some(record)
    }
}

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u32_at(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    let chunk = bytes.get(at..at + 4).expect("u32 within record");
    read_u32_chunk(chunk)
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let chunk = bytes.get(at..at + 8).expect("u64 within record");
    u64::from_le_bytes([
        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
    ])
}

#[cfg(test)]
mod tests {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal, VehicleProfileOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    use super::*;
    use crate::{
        CommittedNetworkSource, RouteRegisterInput, TickInput, TrafficWorld, VehicleHandle,
        VehicleSpawnInput, WorldConfig,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
    );

    fn world() -> TrafficWorld {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        let origin = *revision.canonical_origin();
        TrafficWorld::install(
            revision,
            WorldConfig::new(8, 4, 1_024, 1, 100),
            CommittedNetworkSource::Published {
                reference: crate::PublishedLfcaReference::new(
                    "fixture://migration-journal",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("non-empty fixture key"),
            },
            0,
        )
        .expect("install")
    }

    /// 无路口、无停止线的首边 + 可行后继：与 tick.rs preview 同款选边法。
    fn preview_route(world: &mut TrafficWorld) -> RouteHandle {
        let traffic = world.traffic();
        let mut edges = Vec::new();
        let count = traffic.lane_edge_count();
        for raw in 0..count {
            let edge = LaneEdgeOrdinal::from_raw(raw);
            if traffic.relations().lane_edge_junction(edge).is_some() {
                continue;
            }
            if traffic.relations().stop_line_for_edge(edge).is_some() {
                continue;
            }
            edges.push(edge);
            if let Some(succ) = traffic
                .successors(edge)
                .and_then(|items| items.first().copied())
                && traffic.relations().stop_line_for_edge(succ).is_none()
            {
                edges.push(succ);
            }
            break;
        }
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("preview route")
    }

    fn decoded(world: &TrafficWorld) -> Vec<JournalRecord<'_>> {
        world
            .migration_journal()
            .expect("armed journal")
            .records_from(0)
            .collect()
    }

    fn sample_delta(slot: u32, generation: u32) -> VehicleDelta {
        VehicleDelta {
            slot,
            generation,
            profile: 0,
            class: 2,
            route_index: 4,
            route_generation: 9,
            route_edge_index: 1,
            progress_mm: 1_234,
            carry_um: 567,
            speed_mm_s: 8_900,
            length_mm: 4_500,
            status: VehicleStatus::Active,
            parking: None,
        }
    }

    #[test]
    fn records_from_resumes_at_record_boundary() {
        let mut journal = MigrationDeltaJournal::arm(4_096, 5).expect("arm");
        let delta = sample_delta(3, 1);
        journal.begin_tick(7, 700);
        journal.tick_entry(&delta);
        journal.finish_tick();
        journal.begin_tick(8, 800);
        journal.finish_tick();
        journal.record_route_removed(7, 2, true, 5);

        let full: Vec<_> = journal.records_from(0).collect();
        let mut head = journal.records_from(0);
        assert!(head.next().is_some());
        let offset = head.offset();
        let tail: Vec<_> = journal.records_from(offset).collect();
        assert_eq!(tail.len(), full.len() - 1);
        assert_eq!(tail.as_slice(), &full[1..]);
        // 耗尽后的偏移即写入字节数。
        let mut rest = journal.records_from(offset);
        while rest.next().is_some() {}
        assert_eq!(
            rest.offset(),
            usize::try_from(journal.stats().written_bytes).unwrap()
        );
    }

    #[test]
    fn journal_record_roundtrip_all_kinds() {
        let mut journal = MigrationDeltaJournal::arm(4_096, 5).expect("arm");
        let delta = sample_delta(3, 1);
        journal.begin_tick(7, 700);
        journal.tick_entry(&delta);
        journal.finish_tick();
        journal.begin_tick(8, 800);
        journal.finish_tick();
        journal.record_route_registered(
            6,
            RouteHandle::new(2, 4),
            &[LaneEdgeOrdinal::from_raw(11), LaneEdgeOrdinal::from_raw(12)],
        );
        journal.record_route_removed(7, 2, true, 5);
        journal.record_vehicle_spawned(8, sample_delta(6, 0));
        journal.record_vehicle_replaced(9, VehicleHandle::new(6, 0), 2, sample_delta(6, 1));
        journal.record_parking_occupied(10, VehicleHandle::new(6, 1), 3);

        let records: Vec<_> = journal.records_from(0).collect();
        assert_eq!(records.len(), 7);
        let JournalRecord::Tick {
            tick_index,
            time_ms,
            entries,
        } = records[0]
        else {
            panic!("expected tick record");
        };
        assert_eq!((tick_index, time_ms), (7, 700));
        assert_eq!(entries.len(), VEHICLE_DELTA_BYTES);
        assert_eq!(VehicleDelta::decode(entries), delta);
        assert!(matches!(
            records[1],
            JournalRecord::Tick { tick_index: 8, time_ms: 800, entries } if entries.is_empty()
        ));
        let JournalRecord::RouteRegistered {
            command_cursor,
            slot,
            generation,
            edges,
        } = records[2]
        else {
            panic!("expected route record");
        };
        assert_eq!((command_cursor, slot, generation), (6, 2, 4));
        assert_eq!(raw_u32_stream(edges).collect::<Vec<_>>(), vec![11, 12]);
        assert_eq!(
            records[3],
            JournalRecord::RouteRemoved {
                command_cursor: 7,
                slot: 2,
                recyclable: true,
                generation_after: 5,
            }
        );
        assert!(matches!(
            records[4],
            JournalRecord::VehicleSpawned { command_cursor: 8, ref vehicle }
                if vehicle.slot == 6 && vehicle.generation == 0
        ));
        assert!(matches!(
            records[5],
            JournalRecord::VehicleReplaced { command_cursor: 9, old_slot: 6, old_generation: 0, order_index: 2, ref vehicle }
                if vehicle.generation == 1
        ));
        assert_eq!(
            records[6],
            JournalRecord::ParkingOccupied {
                command_cursor: 10,
                slot: 6,
                generation: 1,
                space: 3,
            }
        );
        assert_eq!(journal.first_tick(), Some(7));
        assert_eq!(journal.last_tick(), Some(8));
        assert_eq!(journal.record_count(), 7);
        assert_eq!(journal.baseline_command_cursor(), 5);
        assert!(!journal.overflowed());
    }

    #[test]
    fn overflow_is_sticky_and_stops_writes() {
        let bound = u64::try_from(TICK_HEADER_BYTES).expect("bound fits u64");
        let mut journal = MigrationDeltaJournal::arm(bound, 0).expect("arm");
        journal.begin_tick(1, 100);
        journal.tick_entry(&sample_delta(0, 0));
        journal.finish_tick();
        assert!(journal.overflowed());
        assert_eq!(journal.written_bytes(), bound);
        assert_eq!(journal.record_count(), 1);
        // 粘性：后续记录全部丢弃，统计不再推进。
        journal.begin_tick(2, 200);
        journal.finish_tick();
        journal.record_parking_occupied(1, VehicleHandle::new(0, 0), 0);
        assert_eq!(journal.record_count(), 1);
        assert_eq!(journal.last_tick(), Some(1));
    }

    #[test]
    fn armed_journal_captures_scripted_evolution() {
        let mut world = world();
        let route = preview_route(&mut world);
        let profile = world
            .traffic()
            .relations()
            .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
            .expect("profile");
        // 前车按 tick.rs 先例保持安全间距，后车在武装窗口内生成。
        let first = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000 + profile.length_mm() + profile.min_gap_mm() + 2_000,
                0,
            ))
            .expect("leader vehicle");
        // 基线 = 1 次路线注册 + 1 次生成。
        assert_eq!(world.command_cursor(), 2);
        world
            .arm_migration_journal(64 * 1_024)
            .expect("arm journal");

        world.step(TickInput::new(100)).expect("step");
        let second = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("follower vehicle");
        world
            .occupy_parking(second, ParkingSpaceOrdinal::from_raw(0))
            .expect("parking");
        // 停车后 second 不步进：本拍 TICK 只含 first 的条目。
        world.step(TickInput::new(100)).expect("step");
        let parked_tick_entries = {
            let records = decoded(&world);
            match records.last().expect("tick record") {
                JournalRecord::Tick { entries, .. } => entries.len() / VEHICLE_DELTA_BYTES,
                other => panic!("expected tick, got {other:?}"),
            }
        };
        let route_edges: Vec<_> = world.route_edges(route).expect("route").to_vec();
        let temp_route = world
            .register_route(RouteRegisterInput::new(route_edges))
            .expect("temp route");
        world.remove_route(temp_route).expect("remove");
        // 幂等重占不产生记录（无动态变化）。
        world
            .occupy_parking(second, ParkingSpaceOrdinal::from_raw(0))
            .expect("idempotent re-occupy");
        // 强制完成 first 后原子替换。
        let index = usize::try_from(first.index()).expect("index");
        world.vehicles[index].state.as_mut().expect("first").status = VehicleStatus::Completed;
        world
            .replace_completed_vehicle(
                first,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 1_000, 0),
            )
            .expect("replace");

        let records = decoded(&world);
        // 期望序：TICK, SPAWNED, PARKING, TICK, ROUTE_REGISTERED, ROUTE_REMOVED, VEHICLE_REPLACED。
        assert_eq!(records.len(), 7);
        assert!(matches!(
            records[0],
            JournalRecord::Tick {
                tick_index: 1,
                time_ms: 100,
                ..
            }
        ));
        assert!(matches!(
            records[1],
            JournalRecord::VehicleSpawned { command_cursor: 3, ref vehicle }
                if vehicle.slot == second.index() && vehicle.parking.is_none()
        ));
        assert!(matches!(
            records[2],
            JournalRecord::ParkingOccupied { command_cursor: 4, slot, generation, space: 0 }
                if slot == second.index() && generation == second.generation()
        ));
        assert!(matches!(
            records[3],
            JournalRecord::Tick {
                tick_index: 2,
                time_ms: 200,
                ..
            }
        ));
        assert_eq!(
            parked_tick_entries, 1,
            "parked vehicle excluded from tick entries"
        );
        assert!(matches!(
            records[4],
            JournalRecord::RouteRegistered {
                command_cursor: 5,
                ..
            }
        ));
        assert!(matches!(
            records[5],
            JournalRecord::RouteRemoved { command_cursor: 6, slot, .. } if slot == temp_route.index()
        ));
        assert!(matches!(
            records[6],
            JournalRecord::VehicleReplaced { command_cursor: 8, old_slot, order_index: 0, .. }
                if old_slot == first.index()
        ));
        let journal = world.migration_journal().expect("armed");
        assert_eq!(journal.baseline_command_cursor(), 2);
        assert_eq!(journal.first_tick(), Some(1));
        assert_eq!(journal.last_tick(), Some(2));
        assert!(!journal.overflowed());
    }

    #[test]
    fn zero_change_step_still_emits_empty_tick() {
        let mut world = world();
        let route = preview_route(&mut world);
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("vehicle");
        world
            .occupy_parking(vehicle, ParkingSpaceOrdinal::from_raw(0))
            .expect("parking");
        world.arm_migration_journal(4_096).expect("arm");
        world.step(TickInput::new(100)).expect("step");
        let records = decoded(&world);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0],
            JournalRecord::Tick { tick_index: 1, time_ms: 100, entries } if entries.is_empty()
        ));
    }

    #[test]
    fn armed_steady_steps_do_not_grow_arena() {
        let mut world = world();
        let route = preview_route(&mut world);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("vehicle");
        world.arm_migration_journal(4_096).expect("arm");
        let bound = 4_096_usize;
        for _ in 0..16 {
            world.step(TickInput::new(100)).expect("step");
            let journal = world.migration_journal().expect("armed");
            assert!(usize::try_from(journal.written_bytes()).expect("fits") <= bound);
        }
    }

    #[test]
    fn overflowed_journal_keeps_world_stepping() {
        let mut world = world();
        let route = preview_route(&mut world);
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("vehicle");
        // 只够一条 TICK 头：首条 step 即溢出。
        world.arm_migration_journal(21).expect("arm");
        world
            .step(TickInput::new(100))
            .expect("step despite overflow");
        assert!(world.migration_journal().expect("armed").overflowed());
        let after_first = world.vehicle_state(vehicle).expect("vehicle").progress_mm();
        for _ in 0..4 {
            world
                .step(TickInput::new(100))
                .expect("world keeps stepping");
        }
        assert!(world.migration_journal().expect("armed").overflowed());
        assert!(world.vehicle_state(vehicle).expect("vehicle").progress_mm() > after_first);
        assert_eq!(world.tick_index(), 5);
    }

    #[test]
    fn arm_twice_fails_and_disarm_takes_journal() {
        let mut world = world();
        assert!(world.arm_migration_journal(4_096).is_ok());
        assert_eq!(
            world.arm_migration_journal(4_096).unwrap_err(),
            MigrationJournalError::AlreadyArmed
        );
        assert!(world.disarm_migration_journal().is_some());
        assert!(world.migration_journal().is_none());
        assert!(world.disarm_migration_journal().is_none());
    }
}
