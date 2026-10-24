use std::collections::BTreeMap;

use laneflow_runtime::*;
use laneflow_static_contract::{
    EntityKind, ManeuverGateOrdinal, ParkingFacilityOrdinal, SignalAspect, WaitingZoneOrdinal,
};
use serde_json::{Value, json};

use crate::fastmap::FastMap;
use crate::{Artifacts, Harness, IndividualId, Result, digest::Domain, invalid};

// This encoding belongs to the harness v1 observation, not a Runtime wire format.
struct Hash {
    digest: blake3::Hasher,
    pending: [u8; 256],
    used: usize,
}

impl Default for Hash {
    fn default() -> Self {
        Self {
            digest: Domain::Observation.hasher(),
            pending: [0; 256],
            used: 0,
        }
    }
}

impl Hash {
    // 只合批送入相同字节，不改变观测编码、摘要域或检查频率。
    fn bytes(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.pending.len() {
            self.flush();
            self.digest.update(bytes);
            return;
        }
        if self.used + bytes.len() > self.pending.len() {
            self.flush();
        }
        self.pending[self.used..self.used + bytes.len()].copy_from_slice(bytes);
        self.used += bytes.len();
    }

    fn flush(&mut self) {
        if self.used != 0 {
            self.digest.update(&self.pending[..self.used]);
            self.used = 0;
        }
    }

    fn n(&mut self, value: impl Into<u64>) {
        self.bytes(&value.into().to_le_bytes());
    }
    fn text(&mut self, text: &str) {
        self.n(text.len() as u64);
        self.bytes(text.as_bytes());
    }
    fn id(&mut self, id: IndividualId) {
        self.n(id.tile);
        self.n(id.slot);
        self.n(id.incarnation);
    }
    fn finish(mut self) -> String {
        self.flush();
        crate::hex(self.digest.finalize().as_bytes())
    }
}

/// v3 树形摘要的固定块界（个体槽位数 / 决策条数）：与 workers 无关，
/// 保证不同并行度下摘要逐字节一致。
const OBSERVATION_BLOCK: usize = 4096;

/// 逐拍状态摘要的编码版本；存档清单记录它，续跑要求相同。
pub(crate) const OBSERVATION_VERSION: &str = "urban-observation-v4";

/// 车身区间：(边 raw, 车尾 µm, 车头 µm, 个体)。
type Body = (u32, u64, u64, IndividualId);

/// 逐拍观测的跨拍缓冲区：每个固定块一份，各阶段依次复用；只保留容量，
/// 内容逐拍清空，不参与摘要与检查语义。
#[derive(Default)]
pub(crate) struct ObservationScratch {
    blocks: Vec<std::sync::Mutex<BlockBuffers>>,
    bodies: Vec<Body>,
    merge: Vec<Body>,
    runs: Vec<usize>,
    claims: Vec<(IndividualId, u32)>,
    zone_owners: FastMap<u32, IndividualId>,
}

#[derive(Default)]
struct BlockBuffers {
    bytes: Vec<u8>,
    json: Vec<u8>,
    bodies: Vec<Body>,
    states: Vec<Option<VehicleState>>,
}

impl ObservationScratch {
    /// 保证至少 `len` 个块缓冲区（只增不减）。
    fn blocks(&mut self, len: usize, block_size: usize) -> &[std::sync::Mutex<BlockBuffers>] {
        let count = len.div_ceil(block_size);
        if self.blocks.len() < count {
            self.blocks.resize_with(count, Default::default);
        }
        &self.blocks
    }
}

fn block_buffers(
    blocks: &[std::sync::Mutex<BlockBuffers>],
    from: usize,
    block_size: usize,
) -> std::sync::MutexGuard<'_, BlockBuffers> {
    blocks[from / block_size]
        .lock()
        .expect("observation block buffers mutex poisoned")
}

/// state() 逐车并行的块编码器：字节流与 Hash 的 n/text/id/json 完全相同。
/// 每块字节在块线程内各自取 BLAKE3-256 叶摘要（v4 起；v3 为 SHA-256），主线程按块序把叶摘要
/// 送入根哈希（有单元测试比对）。
struct BlockHash {
    bytes: Vec<u8>,
    scratch: Vec<u8>,
}

impl BlockHash {
    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            scratch: Vec::new(),
        }
    }
    /// 取块缓冲区编码：清空后复用上一拍容量，逐拍不再新分配块字节。
    fn reuse(buffers: &mut BlockBuffers, capacity: usize) -> Self {
        let mut bytes = std::mem::take(&mut buffers.bytes);
        bytes.clear();
        bytes.reserve(capacity);
        Self {
            bytes,
            scratch: std::mem::take(&mut buffers.json),
        }
    }
    /// 取叶摘要并把缓冲区还给块缓冲区。
    fn finish_into(self, buffers: &mut BlockBuffers) -> [u8; 32] {
        let digest = self.digest();
        buffers.bytes = self.bytes;
        buffers.json = self.scratch;
        digest
    }
    fn n(&mut self, value: impl Into<u64>) {
        self.bytes.extend_from_slice(&value.into().to_le_bytes());
    }
    fn text(&mut self, text: &str) {
        self.n(text.len() as u64);
        self.bytes.extend_from_slice(text.as_bytes());
    }
    fn id(&mut self, id: IndividualId) {
        self.n(id.tile);
        self.n(id.slot);
        self.n(id.incarnation);
    }
    fn json<T: serde::Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.scratch.clear();
        serde_json::to_writer(&mut self.scratch, value)?;
        let len = self.scratch.len() as u64;
        self.n(len);
        self.bytes.extend_from_slice(&self.scratch);
        Ok(())
    }
    /// 块字节的叶摘要（[`Domain::ObservationLeaf`]），保留原始字节数组供根哈希直接消费。
    fn digest(&self) -> [u8; 32] {
        Domain::ObservationLeaf.bytes(&self.bytes)
    }
}

/// 固定块界并行执行块函数：[0, len) 按 block_size 切块，常驻池线程按跨步取块
/// 保持负载均衡；pool 为 None（单 worker）或单块时在调用线程上跑，不借用辅助
/// 线程。返回逐块结果，块序即输入序；报错等价于块内序 + 块序首个 Err；块内
/// panic 由 in_place_scope 统一传播，等价于逐块 join 的 expect。
fn run_blocks<T: Send>(
    pool: Option<&rayon_core::ThreadPool>,
    len: usize,
    block_size: usize,
    run: impl Fn(usize, usize) -> Result<T> + Sync,
) -> Vec<Result<T>> {
    debug_assert!(block_size > 0);
    let block_count = len.div_ceil(block_size);
    let mut outcomes: Vec<Option<Result<T>>> = Vec::new();
    outcomes.resize_with(block_count, || None);
    if let Some(pool) = pool.filter(|_| block_count > 1) {
        let tasks = pool.current_num_threads().min(block_count);
        let run = &run;
        let assigned = std::sync::Mutex::new(Vec::new());
        pool.in_place_scope(|scope| {
            let assigned = &assigned;
            for task in 0..tasks {
                scope.spawn(move |_| {
                    let mut done = Vec::new();
                    let mut block = task;
                    while block < block_count {
                        let from = block * block_size;
                        done.push((block, run(from, (from + block_size).min(len))));
                        block += tasks;
                    }
                    assigned
                        .lock()
                        .expect("observation blocks mutex poisoned")
                        .extend(done);
                });
            }
        });
        for (block, outcome) in assigned
            .into_inner()
            .expect("observation blocks mutex poisoned")
        {
            outcomes[block] = Some(outcome);
        }
    } else {
        for (block, slot) in outcomes.iter_mut().enumerate() {
            let from = block * block_size;
            *slot = Some(run(from, (from + block_size).min(len)));
        }
    }
    outcomes
        .into_iter()
        .map(|outcome| outcome.expect("each block computed"))
        .collect()
}

pub(crate) fn transition_gates(a: &Artifacts) -> FastMap<(u32, u32), (u32, u32)> {
    let mut result = FastMap::default();
    for raw in 0..a.revision.identity().entity_count(EntityKind::ManeuverGate) {
        let gate = a
            .revision
            .traffic()
            .relations()
            .maneuver_gate(ManeuverGateOrdinal::from_raw(raw))
            .expect("checked gate");
        if let Some(group) = gate.signal_group() {
            let path = a
                .revision
                .traffic()
                .maneuvers()
                .maneuver_path(gate.path())
                .expect("checked path");
            let hop = gate.transition_index() as usize;
            result.insert(
                (path.edges()[hop].raw(), path.edges()[hop + 1].raw()),
                (raw, group.raw()),
            );
        }
    }
    result
}

// counts() 与 state() 共用同一生命周期/绑定匹配矩阵，避免冷热路径语义漂移。
fn binding_matches_status(status: VehicleStatus, binding: Option<ParkingBinding>) -> bool {
    matches!(
        (status, binding),
        (VehicleStatus::Parked, Some(ParkingBinding::Occupied(_)))
            | (VehicleStatus::Active, Some(ParkingBinding::Reserved(_)))
            | (VehicleStatus::Active | VehicleStatus::Completed, None)
    )
}

pub(crate) fn counts(h: &Harness<'_>) -> Result<(usize, usize, usize)> {
    let live = h
        .individuals
        .iter()
        .filter(|individual| individual.handle.is_some())
        .count();
    if h.world.live_vehicles().len() != live || h.slots.len() != live {
        return Err(invalid("live identity conservation failed"));
    }
    let expected_live = i64::from(h.plan.individuals) + h.births as i64 - h.removals as i64;
    if expected_live < 0 || live != expected_live as usize {
        return Err(invalid("birth/removal identity conservation failed"));
    }
    let mut counts = (0, 0, 0);
    for (index, individual) in h.individuals.iter().enumerate() {
        let Some(handle) = individual.handle else {
            continue;
        };
        if h.slots.get(&handle) != Some(&index) {
            return Err(invalid("handle identity map differs"));
        }
        let state = h
            .world
            .vehicle(handle)
            .ok_or_else(|| invalid("missing live state"))?;
        match state.status() {
            VehicleStatus::Active => counts.0 += 1,
            VehicleStatus::Parked => counts.1 += 1,
            VehicleStatus::Completed => counts.2 += 1,
        }
        if !binding_matches_status(state.status(), h.world.parking_binding(handle)) {
            return Err(invalid("parking binding and lifecycle disagree"));
        }
    }
    Ok(counts)
}

// parking_invariants() 与 state() 共用同一逐车绑定汇总与车位归属检查。
fn accumulate_parking_binding(
    world: &laneflow_runtime::TrafficWorld,
    bindings: &mut FastMap<ParkingTarget, (u64, u64)>,
    handle: VehicleHandle,
    binding: Option<ParkingBinding>,
) -> Result<()> {
    let Some(binding) = binding else {
        return Ok(());
    };
    let (target, reserved) = match binding {
        ParkingBinding::Reserved(r) => (r.target(), true),
        ParkingBinding::Occupied(t) => (t, false),
    };
    let count = bindings.entry(target).or_default();
    if reserved {
        count.0 += 1;
    } else {
        count.1 += 1;
    }
    if let ParkingTarget::ExplicitSpace(space) = target {
        let expected = if reserved {
            ParkingSpaceState::Reserved(handle)
        } else {
            ParkingSpaceState::Occupied(handle)
        };
        if world.parking_space_state(space) != Some(expected) {
            return Err(invalid("space owner differs from binding"));
        }
    }
    Ok(())
}

pub(crate) fn parking_invariants(h: &Harness<'_>) -> Result<()> {
    let mut bindings: FastMap<ParkingTarget, (u64, u64)> = FastMap::default();
    for individual in &h.individuals {
        let Some(handle) = individual.handle else {
            continue;
        };
        accumulate_parking_binding(
            &h.world,
            &mut bindings,
            handle,
            h.world.parking_binding(handle),
        )?;
    }
    for (target, key) in &h.target_keys {
        let (reserved, occupied) = bindings.get(target).copied().unwrap_or_default();
        if let ParkingTarget::ExplicitSpace(space) = target
            && (reserved + occupied > 1
                || (reserved + occupied == 0
                    && h.world.parking_space_state(*space) != Some(ParkingSpaceState::Vacant)))
        {
            return Err(invalid(format!(
                "explicit capacity or ownership differs: {key}"
            )));
        }
    }
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::ParkingFacility)
    {
        let facility = ParkingFacilityOrdinal::from_raw(raw);
        let definition = h
            .artifacts
            .revision
            .traffic()
            .relations()
            .parking_facility(facility)
            .ok_or_else(|| invalid("missing facility definition"))?;
        let explicit = definition
            .spaces()
            .iter()
            .fold((0, 0), |(reserved, occupied), space| {
                let count = bindings
                    .get(&ParkingTarget::ExplicitSpace(*space))
                    .copied()
                    .unwrap_or_default();
                (reserved + count.0, occupied + count.1)
            });
        let virtual_pool = bindings
            .get(&ParkingTarget::VirtualPool(facility))
            .copied()
            .unwrap_or_default();
        let counts = h
            .world
            .parking_facility_counts(facility)
            .ok_or_else(|| invalid("missing facility counts"))?;
        validate_facility_counts(counts, explicit, virtual_pool)?;
    }
    Ok(())
}

fn validate_facility_counts(
    counts: ParkingFacilityCounts,
    explicit: (u64, u64),
    virtual_pool: (u64, u64),
) -> Result<()> {
    if (counts.explicit.reserved, counts.explicit.occupied) != explicit {
        return Err(invalid("explicit facility binding count differs"));
    }
    if (counts.virtual_pool.reserved, counts.virtual_pool.occupied) != virtual_pool
        || (counts.total.reserved, counts.total.occupied)
            != (explicit.0 + virtual_pool.0, explicit.1 + virtual_pool.1)
    {
        return Err(invalid("facility binding count differs"));
    }
    for pool in [counts.explicit, counts.virtual_pool, counts.total] {
        if pool.reserved + pool.occupied + pool.vacant != pool.capacity {
            return Err(invalid("parking capacity conservation failed"));
        }
    }
    Ok(())
}

/// step 前一遍完成：step_before 快照、intent 计数与 red_waiters 采集。
/// 按固定块并行：块内逐车读 world.vehicle 复制快照，主线程按块序拼接
/// step_before（槽位序不变）；intent 求和、red_waiters 求并集，均与
/// 串行结果相同。
pub(crate) fn pre_step(h: &mut Harness<'_>) -> usize {
    let warm = h.world.tick_index() >= h.plan.window.warm_up_ticks;
    h.scratch_red.clear();
    if warm {
        let signals = h.world.committed_signal_groups();
        h.scratch_red.extend(
            signals
                .as_slice()
                .iter()
                .map(|(g, a)| (g.raw(), *a == SignalAspect::Red)),
        );
    }
    let view = StateView {
        world: &h.world,
        individuals: &h.individuals,
        step_before: &[],
        slots: &h.slots,
        route_table: &h.route_table,
        target_keys: &h.target_keys,
        plan: h.plan,
        transition_gates: &h.transition_gates,
        scratch_red: &h.scratch_red,
    };
    let n = h.individuals.len();
    let buffers = h.observation.blocks(n, OBSERVATION_BLOCK);
    let blocks = run_blocks(h.pool.as_ref(), n, OBSERVATION_BLOCK, |from, to| {
        let mut buffers = block_buffers(buffers, from, OBSERVATION_BLOCK);
        Ok(pre_step_block(&view, from, to, warm, &mut buffers.states))
    });
    h.step_before.clear();
    h.step_before.reserve(n);
    let mut intent = 0;
    for (index, block) in blocks.into_iter().enumerate() {
        let block = block.expect("pre_step block encoding is infallible");
        intent += block.intent;
        h.step_before.extend_from_slice(
            &buffers[index]
                .lock()
                .expect("observation block buffers mutex poisoned")
                .states,
        );
        for red in block.red {
            h.red_waiters.insert(red);
        }
    }
    intent
}

/// pre_step 的只读块产出：intent 计数与 red_waiter 候选（主线程求并集，
/// 与串行插入得到的集合相同）；槽位区间的车辆快照写入块缓冲区 `states`。
struct PreBlock {
    intent: usize,
    red: Vec<(IndividualId, u32)>,
}

fn pre_step_block(
    view: &StateView<'_>,
    from: usize,
    to: usize,
    warm: bool,
    states: &mut Vec<Option<VehicleState>>,
) -> PreBlock {
    let mut out = PreBlock {
        intent: 0,
        red: Vec::new(),
    };
    states.clear();
    states.reserve(to - from);
    for i in from..to {
        let state = view.individuals[i]
            .handle
            .and_then(|handle| view.world.vehicle(handle));
        if let Some(state) = state
            && state.status() == VehicleStatus::Active
        {
            out.intent += 1;
            if warm && state.speed_mm_s() == 0 {
                let route = view.route_table.index(state.route());
                let current = state.route_edge_index() as usize;
                if current + 1 < view.route_table.edge_count(route)
                    && view
                        .route_table
                        .edge_len_mm(route, current)
                        .saturating_sub(state.progress_mm())
                        <= 1
                    && let Some(&(gate, group)) = view.transition_gates.get(&(
                        view.route_table.edge_raw(route, current),
                        view.route_table.edge_raw(route, current + 1),
                    ))
                    && view.scratch_red.get(&group) == Some(&true)
                {
                    out.red.push((view.individuals[i].id, gate));
                }
            }
        }
        states.push(state);
    }
    out
}

fn conflict_reason(reason: ConflictNoGrantReason) -> u32 {
    match reason {
        ConflictNoGrantReason::WaitingCapacity => 0,
        ConflictNoGrantReason::WaitingPhysicalStorage => 1,
        ConflictNoGrantReason::WaitingCycle => 2,
        ConflictNoGrantReason::ConflictOccupied => 3,
        ConflictNoGrantReason::LagGap => 4,
        ConflictNoGrantReason::ApproachUnprovable => 5,
        ConflictNoGrantReason::LeadGap => 6,
        ConflictNoGrantReason::DownstreamStorageBoundary => 7,
        ConflictNoGrantReason::DownstreamClaimConflict => 8,
    }
}

fn conflict_reason_name(reason: ConflictNoGrantReason) -> &'static str {
    match reason {
        ConflictNoGrantReason::WaitingCapacity => "waiting-capacity",
        ConflictNoGrantReason::WaitingPhysicalStorage => "waiting-storage",
        ConflictNoGrantReason::WaitingCycle => "waiting-cycle",
        ConflictNoGrantReason::ConflictOccupied => "conflict-occupied",
        ConflictNoGrantReason::LagGap => "lag-gap",
        ConflictNoGrantReason::ApproachUnprovable => "approach-unprovable",
        ConflictNoGrantReason::LeadGap => "lead-gap",
        ConflictNoGrantReason::DownstreamStorageBoundary => "downstream-storage",
        ConflictNoGrantReason::DownstreamClaimConflict => "downstream-claim",
    }
}

/// 通行位置的 JSON 数组字段：元组序列化与 `json!([...])` 字节相同，
/// 逐拍摘要直接编码元组，不为每条决策构造 `Value`。
fn passage_fields(
    route_table: &crate::route_table::RouteTable,
    p: ConflictPassageOccurrenceLocator,
) -> impl serde::Serialize + '_ {
    (
        route_table.key_of(p.route()),
        p.maneuver_occurrence_index(),
        p.admission_gate_hop(),
        p.conflict_occurrence_index(),
        p.address().zone().raw(),
        p.address().stream().raw(),
        p.address().passage_local_index(),
    )
}
fn passage(
    route_table: &crate::route_table::RouteTable,
    p: ConflictPassageOccurrenceLocator,
) -> Value {
    json!(passage_fields(route_table, p))
}
fn range(route_table: &crate::route_table::RouteTable, r: ConflictPassageRange) -> Value {
    json!(range_fields(route_table, r))
}
fn range_fields(
    route_table: &crate::route_table::RouteTable,
    r: ConflictPassageRange,
) -> impl serde::Serialize + '_ {
    (
        route_table.key_of(r.route()),
        r.maneuver_occurrence_index(),
        r.admission_gate_hop(),
        r.first_conflict_occurrence_index(),
        r.passage_count(),
    )
}

// Match an actual tick-start claim to this subject's declared yield target. Generic
// resource rejection, an active pulse elsewhere, or a downstream blockage is not proof.
fn occupied_yield_pulse(
    h: &Harness<'_>,
    decision: ConflictDecision,
    subject: IndividualId,
    pulse_prefix: &str,
) -> Option<crate::runner::RightOfWayWitness> {
    if decision.outcome()
        != ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
    {
        return None;
    }
    let first = decision.passage()?;
    let class = h.world.vehicle(decision.vehicle())?.class();
    let policy = h.world.policy()?;
    // The decision anchors the first passage of the whole admission bundle, not the
    // particular passage that rejected it. Inspect only that same maneuver/gate bundle.
    let passages = (first.conflict_occurrence_index()..)
        .map_while(|index| {
            h.world
                .conflict_passage_occurrence_locator(first.route(), index)
        })
        .take_while(|passage| {
            passage.maneuver_occurrence_index() == first.maneuver_occurrence_index()
                && passage.admission_gate_hop() == first.admission_gate_hop()
        });
    for passage in passages {
        let address = passage.address();
        let (zone, targets) =
            policy.yield_targets(address.stream(), class, address.passage_local_index())?;
        if zone != address.zone() {
            return None;
        }
        let pulse = h
            .claims
            .iter()
            .find_map(|((owner, route, index), claimed)| {
                if owner.tile != subject.tile || *claimed != zone.raw() {
                    return None;
                }
                let slot = owner.tile as usize * 1_000 + owner.slot as usize;
                if h.individuals[slot].id != *owner
                    || !h.plan.initial[slot]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with(pulse_prefix))
                {
                    return None;
                }
                let target = h
                    .world
                    .conflict_passage_occurrence_locator(h.route_table.handle(*route), *index)?
                    .address();
                targets
                    .iter()
                    .any(|cell| {
                        cell.stream() == target.stream()
                            && cell.passage_local_index() == target.passage_local_index()
                    })
                    .then_some(*owner)
            });
        let Some(pulse) = pulse else {
            continue;
        };
        return Some(crate::runner::RightOfWayWitness {
            subject,
            pulse,
            zone: zone.raw(),
            maneuver_occurrence: decision.anchor().maneuver_occurrence_index(),
            occupied_wait: (h.world.tick_index(), decision.vehicle_update_sequence()),
            pulse_cleared: None,
            granted: None,
            passed: None,
        });
    }
    None
}

/// 决策批编码的只读块：块字节与 v2 顺序流对应切片逐字节一致；v3 取块
/// 字节的叶摘要，主线程按块序送入根哈希。
fn waiting_decision_block(
    view: &StateView<'_>,
    from: usize,
    to: usize,
    buffers: &mut BlockBuffers,
) -> Result<[u8; 32]> {
    let mut hash = BlockHash::reuse(buffers, (to - from) * 64);
    for d in &view.world.latest_waiting_decisions()[from..to] {
        let id = view
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        hash.id(id);
        hash.n(d.vehicle_update_sequence());
        hash.text(view.route_table.key_of(d.anchor().route()));
        hash.n(d.anchor().maneuver_occurrence_index());
        hash.n(d.anchor().hop());
        hash.n(d.zone().map_or(u32::MAX, |z| z.raw()));
        let (tag, reason) = match d.outcome() {
            WaitingDecisionOutcome::Deferred => (0_u32, 0),
            WaitingDecisionOutcome::NotEvaluated => (1, 0),
            WaitingDecisionOutcome::NotRequired => (2, 0),
            WaitingDecisionOutcome::Granted => (3, 0),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity) => (4, 0),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage) => (4, 1),
            WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(r)) => {
                (4, 2 + conflict_reason(r))
            }
        };
        hash.n(tag);
        hash.n(reason);
    }
    Ok(hash.finish_into(buffers))
}

fn conflict_decision_block(
    view: &StateView<'_>,
    from: usize,
    to: usize,
    buffers: &mut BlockBuffers,
) -> Result<[u8; 32]> {
    let mut hash = BlockHash::reuse(buffers, (to - from) * 96);
    for d in &view.world.latest_conflict_decisions()[from..to] {
        let id = view
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        hash.id(id);
        hash.n(d.vehicle_update_sequence());
        hash.text(view.route_table.key_of(d.anchor().route()));
        hash.n(d.anchor().maneuver_occurrence_index());
        hash.n(d.anchor().hop());
        hash.json(&d.passage().map(|p| passage_fields(view.route_table, p)))?;
        let (tag, reason) = match d.outcome() {
            ConflictDecisionOutcome::NotEvaluated => (0_u32, 0),
            ConflictDecisionOutcome::NotRequired => (1, 0),
            ConflictDecisionOutcome::Granted => (2, 0),
            ConflictDecisionOutcome::NoGrant(r) => (3, conflict_reason(r)),
        };
        hash.n(tag);
        hash.n(reason);
    }
    Ok(hash.finish_into(buffers))
}

pub(crate) fn events(h: &mut Harness<'_>) -> Result<()> {
    // Every decision, including NotEvaluated/NotRequired, contributes in public batch order.
    // Store its digest once per tick instead of millions of repetitive diagnostic rows.
    // v3 决策批摘要：编码按固定块并行取叶摘要（块界与 workers 无关），主线程
    // 按块序把叶摘要送入根哈希；证据（`evidence`）的 right_of_way 状态机有跨决策顺序
    // 依赖，保持串行第二遍。两遍遍历同一 latest_*_decisions() 快照，顺序相同。
    let tick = h.world.tick_index();
    let mut decisions = Hash::default();
    decisions.text("waiting");
    {
        let view = StateView {
            world: &h.world,
            individuals: &h.individuals,
            step_before: &h.step_before,
            slots: &h.slots,
            route_table: &h.route_table,
            target_keys: &h.target_keys,
            plan: h.plan,
            transition_gates: &h.transition_gates,
            scratch_red: &h.scratch_red,
        };
        let waiting_len = view.world.latest_waiting_decisions().len();
        decisions.n(waiting_len.div_ceil(OBSERVATION_BLOCK) as u64);
        let buffers = h.observation.blocks(waiting_len, OBSERVATION_BLOCK);
        for leaf in run_blocks(
            h.pool.as_ref(),
            waiting_len,
            OBSERVATION_BLOCK,
            |from, to| {
                waiting_decision_block(
                    &view,
                    from,
                    to,
                    &mut block_buffers(buffers, from, OBSERVATION_BLOCK),
                )
            },
        ) {
            decisions.bytes(&leaf?);
        }
        decisions.text("conflict");
        let conflict_len = view.world.latest_conflict_decisions().len();
        decisions.n(conflict_len.div_ceil(OBSERVATION_BLOCK) as u64);
        let buffers = h.observation.blocks(conflict_len, OBSERVATION_BLOCK);
        for leaf in run_blocks(
            h.pool.as_ref(),
            conflict_len,
            OBSERVATION_BLOCK,
            |from, to| {
                conflict_decision_block(
                    &view,
                    from,
                    to,
                    &mut block_buffers(buffers, from, OBSERVATION_BLOCK),
                )
            },
        ) {
            decisions.bytes(&leaf?);
        }
    }
    for d in h.world.latest_waiting_decisions() {
        let id = h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        if h.in_step_observation(tick)
            && h.plan.case == "WAITING-RELEASE"
            && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                .role
                .as_deref()
                .is_some_and(|role| role.starts_with("waiting-fifo-"))
        {
            match d.outcome() {
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::Capacity) => {
                    h.evidence[id.tile as usize].waiting_capacity_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::PhysicalStorage) => {
                    h.evidence[id.tile as usize].waiting_storage_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(
                    ConflictNoGrantReason::WaitingCapacity,
                )) => {
                    h.evidence[id.tile as usize].waiting_capacity_rejections += 1;
                }
                WaitingDecisionOutcome::NoGrant(WaitingNoGrantReason::CombinedResource(
                    ConflictNoGrantReason::WaitingPhysicalStorage
                    | ConflictNoGrantReason::DownstreamStorageBoundary,
                )) => {
                    h.evidence[id.tile as usize].waiting_storage_rejections += 1;
                }
                _ => {}
            }
        }
    }
    for d in h.world.latest_conflict_decisions() {
        let id = h
            .stable_individual(d.vehicle())
            .ok_or_else(|| invalid("unknown decision owner"))?;
        let role = h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
            .role
            .as_deref();
        let pulse_prefix = match (h.plan.case.as_str(), role) {
            ("PERMISSIVE-LEFT", Some("permissive-left")) => Some("opposing-pulse-"),
            ("UNCONTROLLED-YIELD", Some("yield-role")) => Some("mainline-pulse-"),
            _ => None,
        };
        if h.in_step_observation(tick)
            && let Some(prefix) = pulse_prefix
        {
            let witness = h.evidence[id.tile as usize]
                .right_of_way
                .is_none()
                .then(|| occupied_yield_pulse(h, *d, id, prefix))
                .flatten();
            let evidence = &mut h.evidence[id.tile as usize];
            if let Some(witness) = witness {
                if h.plan.case == "PERMISSIVE-LEFT" {
                    evidence.permissive_no_grants += 1;
                } else {
                    evidence.yield_waits += 1;
                }
                evidence.right_of_way = Some(witness);
            }
            match d.outcome() {
                ConflictDecisionOutcome::Granted => {
                    if h.plan.case == "PERMISSIVE-LEFT" {
                        evidence.permissive_grants += 1;
                    }
                    if let Some(witness) = &mut evidence.right_of_way
                        && witness.subject == id
                        && witness.maneuver_occurrence == d.anchor().maneuver_occurrence_index()
                    {
                        witness
                            .granted
                            .get_or_insert((tick, d.vehicle_update_sequence()));
                    }
                }
                ConflictDecisionOutcome::NoGrant(reason) => {
                    *evidence
                        .role_no_grant_reasons
                        .entry(conflict_reason_name(reason).into())
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        if h.in_step_observation(tick)
            && h.plan.case == "WAITING-RELEASE"
            && role.is_some_and(|role| role.starts_with("waiting-fifo-"))
            && matches!(
                d.outcome(),
                ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::DownstreamStorageBoundary)
            )
        {
            h.evidence[id.tile as usize].waiting_storage_rejections += 1;
        }
    }
    h.events.push(json!({"kind":"decision-batch", "tick":tick,
        "waiting_count":h.world.latest_waiting_decisions().len(), "conflict_count":h.world.latest_conflict_decisions().len(), "digest":decisions.finish()}));
    for e in h.world.latest_transition_events() {
        let id = h
            .stable_individual(e.vehicle())
            .ok_or_else(|| invalid("unknown transition owner"))?;
        match e.kind() {
            TrafficTransitionKind::ReservationAcquired { passage_range: r } => {
                for index in r.first_conflict_occurrence_index()
                    ..r.first_conflict_occurrence_index() + r.passage_count()
                {
                    let p = h
                        .world
                        .conflict_passage_occurrence_locator(r.route(), index)
                        .ok_or_else(|| invalid("unknown reservation passage"))?;
                    if h.claims
                        .insert(
                            (id, h.route_table.index(r.route()), index),
                            p.address().zone().raw(),
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate committed claim"));
                    }
                }
            }
            TrafficTransitionKind::ConflictCleared { passage: p } => {
                if h.in_step_observation(e.tick())
                    && let Some(witness) = &mut h.evidence[id.tile as usize].right_of_way
                    && witness.pulse == id
                    && witness.zone == p.address().zone().raw()
                {
                    witness
                        .pulse_cleared
                        .get_or_insert((e.tick(), e.vehicle_update_sequence()));
                }
                if h.claims
                    .remove(&(
                        id,
                        h.route_table.index(p.route()),
                        p.conflict_occurrence_index(),
                    ))
                    .is_none()
                {
                    return Err(invalid("cleared passage has no committed claim"));
                }
            }
            TrafficTransitionKind::ReservationReleased { .. }
                if h.claims.keys().any(|(owner, _, _)| *owner == id) =>
            {
                return Err(invalid("reservation released before all claims cleared"));
            }
            _ => {}
        }
        let detail = match e.kind() {
            TrafficTransitionKind::ProjectionApplied { zone, reason } => json!([
                "projection",
                zone.raw(),
                match reason {
                    WaitingProjectionReason::EvaluationHorizon => "horizon",
                    WaitingProjectionReason::Capacity => "capacity",
                    WaitingProjectionReason::PhysicalStorage => "storage",
                }
            ]),
            TrafficTransitionKind::GateCrossed { gate } => {
                if h.red_waiters.remove(&(id, gate.raw())) {
                    h.evidence[id.tile as usize].red_wait_then_crossed += 1;
                }
                json!(["gate-crossed", gate.raw()])
            }
            TrafficTransitionKind::WaitingLeft {
                zone,
                admission_sequence,
            } => {
                if h.in_step_observation(e.tick())
                    && h.plan.case == "WAITING-RELEASE"
                    && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with("waiting-fifo-"))
                {
                    let evidence = &mut h.evidence[id.tile as usize];
                    evidence.waiting_releases += 1;
                    evidence.waiting_release_order.push(id.slot);
                }
                json!(["waiting-left", zone.raw(), admission_sequence])
            }
            TrafficTransitionKind::WaitingEntered {
                zone,
                admission_sequence,
            } => {
                if h.in_step_observation(e.tick())
                    && h.plan.case == "WAITING-RELEASE"
                    && h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                        .role
                        .as_deref()
                        .is_some_and(|role| role.starts_with("waiting-fifo-"))
                {
                    let evidence = &mut h.evidence[id.tile as usize];
                    evidence.waiting_entries += 1;
                    evidence.waiting_entry_order.push(id.slot);
                }
                json!(["waiting-entered", zone.raw(), admission_sequence])
            }
            TrafficTransitionKind::ReservationAcquired { passage_range } => {
                json!(["reservation-acquired", range(&h.route_table, passage_range)])
            }
            TrafficTransitionKind::ReservationReleased { passage_range } => {
                json!(["reservation-released", range(&h.route_table, passage_range)])
            }
            TrafficTransitionKind::ConflictEntered { passage: p } => {
                json!(["conflict-entered", passage(&h.route_table, p)])
            }
            TrafficTransitionKind::ConflictCleared { passage: p } => {
                json!(["conflict-cleared", passage(&h.route_table, p)])
            }
            TrafficTransitionKind::ManeuverTraversalCompleted {
                maneuver_occurrence_index,
            } => {
                let role = h.plan.initial[id.tile as usize * 1_000 + id.slot as usize]
                    .role
                    .as_deref();
                let in_observation = h.in_step_observation(e.tick());
                let case = h.plan.case.as_str();
                let evidence = &mut h.evidence[id.tile as usize];
                if in_observation
                    && let Some(witness) = &mut evidence.right_of_way
                    && witness.subject == id
                    && witness.maneuver_occurrence == maneuver_occurrence_index
                {
                    witness
                        .passed
                        .get_or_insert((e.tick(), e.vehicle_update_sequence()));
                }
                if in_observation && case == "PERMISSIVE-LEFT" && role == Some("permissive-left") {
                    evidence.permissive_passes += 1;
                }
                if in_observation && case == "UNCONTROLLED-YIELD" {
                    if role.is_some_and(|role| role.starts_with("mainline-pulse-")) {
                        evidence.mainline_passes += 1;
                    } else if role == Some("yield-role") {
                        evidence.yield_passes += 1;
                    }
                }
                json!(["maneuver-completed", maneuver_occurrence_index])
            }
        };
        let a = e.anchor();
        let p = a.position();
        h.events.push(json!({"kind":"transition", "tick":e.tick(), "individual":id, "sequence":e.vehicle_update_sequence(),
            "route":h.route_key(a.route()), "occurrence":a.maneuver_occurrence_index(), "hop":a.hop(),
            "position":[p.route_edge_index(),p.progress_mm(),p.carry_um()], "detail":detail}));
    }
    Ok(())
}

pub(crate) fn status(value: VehicleStatus) -> u32 {
    match value {
        VehicleStatus::Active => 0,
        VehicleStatus::Parked => 1,
        VehicleStatus::Completed => 2,
    }
}

/// state() 逐车循环的单块产出。除编码字节外还携带该块的检查汇总、
/// lifecycle 事件与 crossed_tile 置位请求，由主线程按块序合并：
/// 事件顺序与串行逐车一致，计数类合并满足交换律，crossed_tile 置位幂等。
struct BlockOutcome {
    digest: [u8; 32],
    live: usize,
    status_counts: (usize, usize, usize),
    bindings: FastMap<ParkingTarget, (u64, u64)>,
    waiting: BTreeMap<u32, u32>,
    lifecycle: Vec<Value>,
    crossed: Vec<usize>,
    crossed_completed: Vec<usize>,
}

/// state_block 的只读视图：观测只碰 TrafficWorld（Sync），不碰 Host 枚举
///（adapter feature 下 bevy App 非 Sync），因此块线程可以安全共享。
struct StateView<'x> {
    world: &'x laneflow_runtime::TrafficWorld,
    individuals: &'x [crate::runner::Individual],
    step_before: &'x [Option<VehicleState>],
    slots: &'x FastMap<VehicleHandle, usize>,
    route_table: &'x crate::route_table::RouteTable,
    target_keys: &'x FastMap<ParkingTarget, String>,
    plan: &'x crate::ResolvedPlan,
    transition_gates: &'x FastMap<(u32, u32), (u32, u32)>,
    scratch_red: &'x FastMap<u32, bool>,
}

impl StateView<'_> {
    fn stable_individual(&self, handle: VehicleHandle) -> Option<IndividualId> {
        self.slots.get(&handle).map(|i| self.individuals[*i].id)
    }
}

/// 块车身区间写入块缓冲区 `bodies`（块内已排序），由主线程按块序归并。
fn state_block(
    view: &StateView<'_>,
    from: usize,
    to: usize,
    tick: u64,
    buffers: &mut BlockBuffers,
) -> Result<BlockOutcome> {
    let mut hash = BlockHash::reuse(buffers, (to - from) * 192);
    buffers.bodies.clear();
    let mut out = BlockOutcome {
        digest: [0; 32],
        live: 0,
        status_counts: (0, 0, 0),
        bindings: FastMap::default(),
        waiting: BTreeMap::new(),
        lifecycle: Vec::new(),
        crossed: Vec::new(),
        crossed_completed: Vec::new(),
    };
    for i in from..to {
        // 生命周期比对：与串行版同槽位序；crossed_tile 置位延后到主线程
        // 合并（事件取 旧值 || 本拍跨 tile，与串行置位后再读一致）。
        if let Some(before) = view.step_before[i] {
            let after = view
                .world
                .vehicle(
                    view.individuals[i]
                        .handle
                        .expect("step cannot create an absent slot"),
                )
                .expect("live individual");
            let route = view.route_table.index(after.route());
            let first = before.route_edge_index() as usize;
            let last = after.route_edge_index() as usize;
            let mut crossed_now = false;
            if before.status() == VehicleStatus::Active && first < last {
                for hop in first..last {
                    if view.route_table.crosses_tile(route, hop) {
                        crossed_now = true;
                    }
                }
            }
            if crossed_now {
                out.crossed.push(i);
            }
            if before.status() != after.status() {
                let id = view.individuals[i].id;
                let crossed_tile = view.individuals[i].crossed_tile || crossed_now;
                out.lifecycle.push(json!({"kind":"lifecycle", "phase":"step", "tick":tick, "individual":id,
                    "before":status(before.status()), "after":status(after.status()), "crossed_tile":crossed_tile}));
                if after.status() == VehicleStatus::Completed
                    && crossed_tile
                    && view.plan.window.contains_completed_step(tick)
                {
                    out.crossed_completed.push(id.tile as usize);
                }
            }
        }
        let individual = &view.individuals[i];
        hash.id(individual.id);
        let Some(handle) = individual.handle else {
            hash.n(0_u32);
            continue;
        };
        out.live += 1;
        hash.n(1_u32);
        let v = view
            .world
            .vehicle(handle)
            .ok_or_else(|| invalid("missing vehicle"))?;
        // counts() 的逐车部分（冷路径 counts() 仍供 install 与报告使用）。
        if view.slots.get(&handle) != Some(&i) {
            return Err(invalid("handle identity map differs"));
        }
        match v.status() {
            VehicleStatus::Active => out.status_counts.0 += 1,
            VehicleStatus::Parked => out.status_counts.1 += 1,
            VehicleStatus::Completed => out.status_counts.2 += 1,
        }
        let binding = view.world.parking_binding(handle);
        if !binding_matches_status(v.status(), binding) {
            return Err(invalid("parking binding and lifecycle disagree"));
        }
        accumulate_parking_binding(view.world, &mut out.bindings, handle, binding)?;
        hash.n(v.profile().raw());
        hash.n(v.class().raw());
        hash.text(view.route_table.key_of(v.route()));
        for n in [
            v.route_edge_index(),
            v.progress_mm(),
            u32::from(v.carry_um()),
            v.speed_mm_s(),
            v.length_mm(),
            status(v.status()),
        ] {
            hash.n(n);
        }
        if let Some(t) = v.maneuver_traversal() {
            hash.n(1_u32);
            hash.text(view.route_table.key_of(t.route()));
            hash.n(t.maneuver_occurrence_index());
            let (tag, hop) = match t.phase() {
                ManeuverTraversalPhase::PreGate { next_gate_hop } => (0_u32, next_gate_hop),
                ManeuverTraversalPhase::Committed {
                    last_crossed_gate_hop,
                } => (1, last_crossed_gate_hop),
                ManeuverTraversalPhase::Waiting { release_gate_hop } => (2, release_gate_hop),
                ManeuverTraversalPhase::Clearing { admission_gate_hop } => (3, admission_gate_hop),
            };
            hash.n(tag);
            hash.n(hop);
        } else {
            hash.n(0_u32);
        }
        if let Some(m) = v.waiting_membership() {
            hash.n(1_u32);
            hash.n(m.waiting_zone().raw());
            hash.n(m.admission_sequence());
            hash.n(m.release_hop());
            *out.waiting.entry(m.waiting_zone().raw()).or_default() += 1;
        } else {
            hash.n(0_u32);
        }
        match binding {
            None => hash.n(0_u32),
            Some(ParkingBinding::Occupied(target)) => {
                hash.n(1_u32);
                hash.text(&view.target_keys[&target]);
            }
            Some(ParkingBinding::Reserved(r)) => {
                hash.n(2_u32);
                hash.text(&view.target_keys[&r.target()]);
                hash.text(view.route_table.key_of(r.route()));
                hash.n(r.entry_route_occurrence());
                hash.n(r.virtual_entry_selector().map_or(u32::MAX, |s| s.raw()));
            }
        }
        if let Some(r) = view.world.conflict_reservation(handle) {
            hash.n(1_u32);
            hash.json(&range_fields(view.route_table, r.passage_range()))?;
            hash.id(view
                .stable_individual(r.downstream_owner())
                .ok_or_else(|| invalid("unknown claim owner"))?);
            hash.n(r.downstream_claim_count());
            hash.n(r.acquired_tick());
        } else {
            hash.n(0_u32);
        }
        if v.status() == VehicleStatus::Active {
            let route = view.route_table.index(v.route());
            let mut cursor = v.route_edge_index() as usize;
            let mut front = u64::from(v.progress_mm()) * 1_000 + u64::from(v.carry_um());
            let mut remaining = u64::from(v.length_mm()) * 1_000;
            if front > u64::from(view.route_table.edge_len_mm(route, cursor)) * 1_000 {
                return Err(invalid(format!(
                    "front outside current route edge: tick={} individual={:?} route={} occurrence={} edge={} progress_mm={} carry_um={} length_mm={} speed_mm_s={}",
                    tick,
                    individual.id,
                    view.route_table.key_of(v.route()),
                    v.route_edge_index(),
                    view.route_table.edge_key(route, cursor),
                    v.progress_mm(),
                    v.carry_um(),
                    view.route_table.edge_len_mm(route, cursor),
                    v.speed_mm_s()
                )));
            }
            loop {
                let back = front.saturating_sub(remaining);
                if front > back {
                    buffers.bodies.push((
                        view.route_table.edge_raw(route, cursor),
                        back,
                        front,
                        individual.id,
                    ));
                }
                remaining = remaining.saturating_sub(front);
                if remaining == 0 || cursor == 0 {
                    break;
                }
                cursor -= 1;
                front = u64::from(view.route_table.edge_len_mm(route, cursor)) * 1_000;
            }
        }
    }
    buffers.bodies.sort_unstable();
    out.digest = hash.finish_into(buffers);
    Ok(out)
}

pub(crate) fn state(h: &mut Harness<'_>) -> Result<(String, (usize, usize, usize))> {
    state_impl(h, OBSERVATION_BLOCK)
}

/// 资源声明（`claims`）不变量的只读块检查：按原 map 迭代序逐条核对预约持有方，
/// 返回块内首个缺失持有方的位置；主线程按块序取全局首个位置，再按 map 序
/// 逐条先插入 zone_owners 再判定，触发点与串行逐条（zone 独占冲突先于持有方
/// 缺失）一致。
fn claims_block(
    world: &TrafficWorld,
    individuals: &[crate::runner::Individual],
    claims: &[(IndividualId, u32)],
) -> Option<usize> {
    claims.iter().position(|(id, _)| {
        let individual = &individuals[id.tile as usize * 1_000 + id.slot as usize];
        individual.id != *id
            || individual
                .handle
                .and_then(|handle| world.conflict_reservation(handle))
                .is_none()
    })
}

/// 把相邻的已排序段（界为 `runs`，首元 0、末元总长）两两归并到 `aux`
/// 并交换，直到只剩一段；与全量排序得到同一总序（元组全序）。
fn merge_sorted_runs(data: &mut Vec<Body>, runs: &mut Vec<usize>, aux: &mut Vec<Body>) {
    while runs.len() > 2 {
        aux.clear();
        let mut write = 1;
        let mut i = 0;
        while i + 2 < runs.len() {
            let (left, right) = (&data[runs[i]..runs[i + 1]], &data[runs[i + 1]..runs[i + 2]]);
            let (mut a, mut b) = (0, 0);
            while a < left.len() && b < right.len() {
                if left[a] <= right[b] {
                    aux.push(left[a]);
                    a += 1;
                } else {
                    aux.push(right[b]);
                    b += 1;
                }
            }
            aux.extend_from_slice(&left[a..]);
            aux.extend_from_slice(&right[b..]);
            runs[write] = runs[i + 2];
            write += 1;
            i += 2;
        }
        if i + 1 < runs.len() {
            aux.extend_from_slice(&data[runs[i]..runs[i + 1]]);
            runs[write] = runs[i + 1];
            write += 1;
        }
        runs.truncate(write);
        std::mem::swap(data, aux);
    }
}

/// 树形摘要（v3 起）：根 = 标识 ‖ 游标 ‖ 块数 ‖ 依块序的叶摘要 ‖ 尾部（等待区、
/// 信号、`remaining-claims`）；叶 = 固定块界槽位区间编码字节的叶摘要（v4 起为 BLAKE3-256）。
/// `remaining-claims` 段的 route 按内部索引编码（注册顺序即目录 key 升序，序号
/// 与 key 一一对应），不再内联 route key 文本。块界与 workers 无关，摘要跨
/// 并行度稳定；报错仍为块内逐车序 + 块间按块序取首个 Err。
fn state_impl(h: &mut Harness<'_>, block_size: usize) -> Result<(String, (usize, usize, usize))> {
    debug_assert!(block_size > 0);
    let mut hash = Hash::default();
    hash.text(OBSERVATION_VERSION);
    hash.n(h.world.tick_index());
    hash.n(h.world.time_ms());
    hash.n(h.world.command_cursor());
    hash.n(h.world.event_cursor());
    // 资源声明不变量按固定块并行核对：块线程只读校验预约持有方，
    // 主线程按 map 序逐条插入 zone_owners 再判定持有方，报错触发点与串行
    // 逐条一致。声明副本与 zone_owners 复用跨拍缓冲区。
    {
        let scratch = &mut h.observation;
        scratch.claims.clear();
        scratch
            .claims
            .extend(h.claims.iter().map(|((id, _, _), zone)| (*id, *zone)));
        let claims = &scratch.claims;
        // adapter feature 下 Host 含 bevy App（非 Sync）：闭包只捕获解引用后的
        // &TrafficWorld 与 &individuals，与 StateView 的共享方式一致。
        let world: &TrafficWorld = &h.world;
        let individuals: &[crate::runner::Individual] = &h.individuals;
        let missing_owner = run_blocks(
            h.pool.as_ref(),
            claims.len(),
            OBSERVATION_BLOCK,
            |from, to| Ok(claims_block(world, individuals, &claims[from..to])),
        )
        .into_iter()
        .enumerate()
        .find_map(|(block, check)| {
            check
                .expect("claims block check is infallible")
                .map(|index| block * OBSERVATION_BLOCK + index)
        });
        scratch.zone_owners.clear();
        for (index, (id, zone)) in claims.iter().enumerate() {
            if scratch
                .zone_owners
                .insert(*zone, *id)
                .is_some_and(|old| old != *id)
            {
                return Err(invalid("committed conflict claims are not exclusive"));
            }
            if missing_owner == Some(index) {
                return Err(invalid("claim has no current reservation owner"));
            }
        }
    }
    // 逐车循环按固定块并行：块界与 workers 无关（叶摘要跨并行度稳定）；
    // 检查汇总、事件与置位请求随块结果按块序合并，与串行逐车序等价。
    let tick = h.world.tick_index();
    let n = h.individuals.len();
    let view = StateView {
        world: &h.world,
        individuals: &h.individuals,
        step_before: &h.step_before,
        slots: &h.slots,
        route_table: &h.route_table,
        target_keys: &h.target_keys,
        plan: h.plan,
        transition_gates: &h.transition_gates,
        scratch_red: &h.scratch_red,
    };
    hash.n(n.div_ceil(block_size) as u64);
    let mut bodies = std::mem::take(&mut h.observation.bodies);
    bodies.clear();
    bodies.reserve(n);
    let mut runs = std::mem::take(&mut h.observation.runs);
    runs.clear();
    runs.push(0);
    let buffers = h.observation.blocks(n, block_size);
    let outcomes = run_blocks(h.pool.as_ref(), n, block_size, |from, to| {
        state_block(
            &view,
            from,
            to,
            tick,
            &mut block_buffers(buffers, from, block_size),
        )
    });
    let mut waiting = BTreeMap::<u32, u32>::new();
    let mut status_counts = (0_usize, 0_usize, 0_usize);
    let mut live = 0_usize;
    let mut bindings: FastMap<ParkingTarget, (u64, u64)> = FastMap::default();
    for (index, outcome) in outcomes.into_iter().enumerate() {
        let outcome = outcome?;
        hash.bytes(&outcome.digest);
        live += outcome.live;
        status_counts.0 += outcome.status_counts.0;
        status_counts.1 += outcome.status_counts.1;
        status_counts.2 += outcome.status_counts.2;
        for (target, count) in outcome.bindings {
            let entry = bindings.entry(target).or_default();
            entry.0 += count.0;
            entry.1 += count.1;
        }
        bodies.extend_from_slice(
            &buffers[index]
                .lock()
                .expect("observation block buffers mutex poisoned")
                .bodies,
        );
        runs.push(bodies.len());
        for (zone, count) in outcome.waiting {
            *waiting.entry(zone).or_default() += count;
        }
        h.events.extend(outcome.lifecycle);
        for i in outcome.crossed {
            h.individuals[i].crossed_tile = true;
        }
        for tile in outcome.crossed_completed {
            h.evidence[tile].crossed_tile_completed += 1;
        }
    }
    // counts() 的守恒汇总（随逐车循环累计完成，仍在同一拍报错）。
    if h.world.live_vehicles().len() != live || h.slots.len() != live {
        return Err(invalid("live identity conservation failed"));
    }
    let expected_live = i64::from(h.plan.individuals) + h.births as i64 - h.removals as i64;
    if expected_live < 0 || live != expected_live as usize {
        return Err(invalid("birth/removal identity conservation failed"));
    }
    // 各块 bodies 已在块线程内有序；主线程按块界两两归并（复用跨拍缓冲区），
    // 总序与全量 sort_unstable 相同（元组全序，稳定性无关）。
    let mut merge = std::mem::take(&mut h.observation.merge);
    merge_sorted_runs(&mut bodies, &mut runs, &mut merge);
    for pair in bodies.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].2 > pair[1].1 && pair[0].3 != pair[1].3 {
            return Err(invalid(format!(
                "body overlap at tick {} on edge {}: {:?} and {:?}",
                h.world.tick_index(),
                pair[0].0,
                pair[0].3,
                pair[1].3
            )));
        }
    }
    h.observation.bodies = bodies;
    h.observation.merge = merge;
    h.observation.runs = runs;
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::WaitingZone)
    {
        let zone = h
            .world
            .waiting_zone(WaitingZoneOrdinal::from_raw(raw))
            .ok_or_else(|| invalid("missing waiting zone"))?;
        if zone.occupancy() != waiting.get(&raw).copied().unwrap_or(0)
            || zone.occupancy() > zone.max_occupancy()
        {
            return Err(invalid("waiting capacity or membership differs"));
        }
        hash.n(raw);
        hash.n(zone.occupancy());
        hash.n(zone.next_admission_sequence());
    }
    for (group, aspect) in h.world.committed_signal_groups().as_slice() {
        hash.n(group.raw());
        hash.n(match aspect {
            SignalAspect::Red => 0_u32,
            SignalAspect::Yellow => 1,
            SignalAspect::Green => 2,
            _ => return Err(invalid("unsupported signal aspect")),
        });
    }
    hash.text("remaining-claims");
    for ((id, route, index), zone) in &h.claims {
        hash.id(*id);
        hash.n(*route);
        hash.n(*index);
        hash.n(*zone);
    }
    // parking_invariants() 的循环后部分（target 归属与设施计数守恒）。
    for (target, key) in &h.target_keys {
        let (reserved, occupied) = bindings.get(target).copied().unwrap_or_default();
        if let ParkingTarget::ExplicitSpace(space) = target
            && (reserved + occupied > 1
                || (reserved + occupied == 0
                    && h.world.parking_space_state(*space) != Some(ParkingSpaceState::Vacant)))
        {
            return Err(invalid(format!(
                "explicit capacity or ownership differs: {key}"
            )));
        }
    }
    for raw in 0..h
        .artifacts
        .revision
        .identity()
        .entity_count(EntityKind::ParkingFacility)
    {
        let facility = ParkingFacilityOrdinal::from_raw(raw);
        let definition = h
            .artifacts
            .revision
            .traffic()
            .relations()
            .parking_facility(facility)
            .ok_or_else(|| invalid("missing facility definition"))?;
        let explicit = definition
            .spaces()
            .iter()
            .fold((0, 0), |(reserved, occupied), space| {
                let count = bindings
                    .get(&ParkingTarget::ExplicitSpace(*space))
                    .copied()
                    .unwrap_or_default();
                (reserved + count.0, occupied + count.1)
            });
        let virtual_pool = bindings
            .get(&ParkingTarget::VirtualPool(facility))
            .copied()
            .unwrap_or_default();
        let counts = h
            .world
            .parking_facility_counts(facility)
            .ok_or_else(|| invalid("missing facility counts"))?;
        validate_facility_counts(counts, explicit, virtual_pool)?;
    }
    Ok((hash.finish(), status_counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_fields_encode_like_the_json_array() {
        // passage_fields / range_fields 以元组代替 json!([...])：摘要字节必须相同。
        let key = "r-7";
        for (a, b, c) in [(0_u32, 0_u32, 0_u16), (1, u32::MAX, u16::MAX), (42, 7, 3)] {
            let mut tuple = BlockHash::with_capacity(0);
            tuple.json(&Some((key, a, b, c))).unwrap();
            tuple.json(&None::<(&str, u32)>).unwrap();
            let mut value = BlockHash::with_capacity(0);
            value.json(&Some(json!([key, a, b, c]))).unwrap();
            value.json(&None::<Value>).unwrap();
            assert_eq!(tuple.bytes, value.bytes);
        }
    }

    #[test]
    fn merged_runs_match_a_full_sort() {
        let id = |slot| IndividualId {
            tile: 0,
            slot,
            incarnation: 0,
        };
        let mut state = 0x9E37_79B9_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let (mut merge, mut runs) = (Vec::new(), Vec::new());
        for lens in [
            vec![],
            vec![0, 0],
            vec![5],
            vec![3, 0, 4],
            vec![2, 7, 1, 9, 4, 0, 6],
        ] {
            let mut data = Vec::new();
            runs.clear();
            runs.push(0);
            for len in lens {
                let start = data.len();
                for _ in 0..len {
                    let r = next();
                    data.push(((r % 3) as u32, r % 5, r % 11, id((r % 4) as u32)));
                }
                data[start..].sort_unstable();
                runs.push(data.len());
            }
            let mut expected = data.clone();
            expected.sort_unstable();
            merge_sorted_runs(&mut data, &mut runs, &mut merge);
            assert_eq!(data, expected);
        }
    }

    #[test]
    fn batched_observation_hash_preserves_the_exact_encoding() {
        assert_eq!(Hash::default().finish(), Domain::Observation.hex(&[]));
        for (numbers, text) in [
            (1, String::new()),
            (31, "短文本".into()),
            (32, "x".into()),
            (33, "y".repeat(255)),
            (64, "z".repeat(256)),
            (65, "a".repeat(257)),
            (3, "b".repeat(1_000)),
        ] {
            let mut hash = Hash::default();
            let mut bytes = Vec::new();
            for number in 0..numbers {
                hash.n(number as u64);
                bytes.extend_from_slice(&(number as u64).to_le_bytes());
            }
            hash.text(&text);
            bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            let id = IndividualId {
                tile: 12,
                slot: 34,
                incarnation: 56,
            };
            hash.id(id);
            for number in [id.tile, id.slot, id.incarnation] {
                bytes.extend_from_slice(&u64::from(number).to_le_bytes());
            }
            hash.n(u64::MAX);
            bytes.extend_from_slice(&u64::MAX.to_le_bytes());
            assert_eq!(hash.finish(), Domain::Observation.hex(&bytes));
        }
    }

    #[test]
    fn block_json_matches_to_string_byte_for_byte() {
        // 与根摘要 `Hash::text` 送入的字节逐字节一致：长度前缀加 `to_string` 字节。
        fn text_bytes(text: &str) -> Vec<u8> {
            let mut bytes = (text.len() as u64).to_le_bytes().to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        fn check(value: &impl serde::Serialize) {
            let expected = text_bytes(&serde_json::to_string(value).unwrap());
            let mut actual = BlockHash::with_capacity(0);
            actual.json(value).unwrap();
            assert_eq!(actual.bytes, expected);
        }
        let none: Option<Value> = None;
        check(&none);
        check(&Some(json!([
            "tile-0001.route.urban-crossing",
            3,
            1,
            2,
            4,
            5,
            6
        ])));
        check(&json!(null));
        check(&json!([]));
        check(&json!({}));
        check(&json!(0));
        check(&json!("短文本\"转义\\\n"));
        check(&json!([1, "two", null, {"nested": [true, false]}, ""]));
        check(&json!({"unicode": "é中𐍈", "control": "\u{1}\u{1f}"}));
        // BlockHash 连续编码多个值时仍逐字节一致。
        let mut expected = Vec::new();
        let mut actual = BlockHash::with_capacity(0);
        for value in [json!("x"), json!({"k": [1, 2]}), json!(null)] {
            expected.extend(text_bytes(&serde_json::to_string(&value).unwrap()));
            actual.json(&value).unwrap();
        }
        assert_eq!(actual.bytes, expected);
    }

    #[test]
    #[ignore = "有界 release 编码对照，不代表城市运行耗时"]
    fn observation_hash_encoding_wall_clock() {
        fn scalar() -> String {
            let mut hash = Domain::Observation.hasher();
            for index in 0..100_000_u64 {
                for value in [index, 1, 2, 3, 4, 5, 6, 7] {
                    hash.update(&std::hint::black_box(value).to_le_bytes());
                }
                let text = "tile-0001.route.urban-crossing";
                hash.update(&(text.len() as u64).to_le_bytes());
                hash.update(text.as_bytes());
            }
            crate::hex(hash.finalize().as_bytes())
        }
        fn batched() -> String {
            let mut hash = Hash::default();
            for index in 0..100_000_u64 {
                for value in [index, 1, 2, 3, 4, 5, 6, 7] {
                    hash.n(std::hint::black_box(value));
                }
                hash.text("tile-0001.route.urban-crossing");
            }
            hash.finish()
        }
        let expected = scalar();
        assert_eq!(batched(), expected);
        type HashRun = fn() -> String;
        for round in 0..6 {
            let candidates: [(&str, HashRun); 2] = if round % 2 == 0 {
                [("scalar", scalar), ("batched", batched)]
            } else {
                [("batched", batched), ("scalar", scalar)]
            };
            for (name, run) in candidates {
                let start = std::time::Instant::now();
                let digest = run();
                let elapsed = start.elapsed();
                assert_eq!(digest, expected);
                eprintln!(
                    "observation-encoding {round} {name}: {} ns {digest}",
                    elapsed.as_nanos()
                );
            }
        }
    }

    #[test]
    fn block_hash_byte_stream_matches_batched_hash() {
        // 同一操作序列：全程走 Hash 与 分块 BlockHash 后按块序送入同一
        // 根哈希，两者摘要逐字节一致（并行 state() 的等价性基础）。
        let id = IndividualId {
            tile: 3,
            slot: 4,
            incarnation: 5,
        };
        let value = json!(["tile-0000.route", 1, null, {"x": [true, "𐍈"]}]);
        let mut full = Hash::default();
        full.n(0_u64);
        full.n(u64::MAX);
        full.text("城市路線 α");
        full.id(id);
        full.text(&serde_json::to_string(&value).unwrap());
        full.n(7_u64);
        let expected = full.finish();
        // 两块拼接：块边界落在任意操作之间。
        for split in 0..=6 {
            let mut first = BlockHash::with_capacity(0);
            let mut second = BlockHash::with_capacity(0);
            let run = |b: &mut BlockHash, from: usize, to: usize| {
                for op in from..to {
                    match op {
                        0 => b.n(0_u64),
                        1 => b.n(u64::MAX),
                        2 => b.text("城市路線 α"),
                        3 => b.id(id),
                        4 => b.json(&value).unwrap(),
                        _ => b.n(7_u64),
                    }
                }
            };
            run(&mut first, 0, split);
            run(&mut second, split, 6);
            let mut stitched = Hash::default();
            stitched.bytes(&first.bytes);
            stitched.bytes(&second.bytes);
            assert_eq!(stitched.finish(), expected, "split: {split}");
        }
    }

    #[test]
    fn binding_status_matrix_matches_counts_semantics() {
        // Reserved 变体只能由 Runtime 内部构造；其三种组合与以下矩阵同属同一
        // matches! 模式表，且被十万辆短窗正常路径覆盖。
        let space = ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0));
        let occupied = Some(ParkingBinding::Occupied(space));
        assert!(binding_matches_status(VehicleStatus::Parked, occupied));
        assert!(!binding_matches_status(VehicleStatus::Active, occupied));
        assert!(!binding_matches_status(VehicleStatus::Completed, occupied));
        assert!(binding_matches_status(VehicleStatus::Active, None));
        assert!(binding_matches_status(VehicleStatus::Completed, None));
        assert!(!binding_matches_status(VehicleStatus::Parked, None));
    }

    #[test]
    fn rejects_wrong_explicit_totals_even_when_capacity_is_conserved() {
        let mut counts = ParkingFacilityCounts {
            explicit: ParkingPoolCounts {
                capacity: 2,
                reserved: 1,
                occupied: 1,
                vacant: 0,
            },
            virtual_pool: ParkingPoolCounts {
                capacity: 3,
                reserved: 0,
                occupied: 1,
                vacant: 2,
            },
            total: ParkingPoolCounts {
                capacity: 5,
                reserved: 1,
                occupied: 2,
                vacant: 2,
            },
        };
        assert!(validate_facility_counts(counts, (1, 1), (0, 1)).is_ok());
        counts.explicit.reserved = 0;
        counts.explicit.occupied = 2;
        counts.total.reserved = 0;
        counts.total.occupied = 3;
        assert!(
            validate_facility_counts(counts, (1, 1), (0, 1))
                .unwrap_err()
                .to_string()
                .contains("explicit facility binding count differs")
        );
    }

    #[test]
    fn block_digest_is_the_leaf_digest_of_block_bytes() {
        let mut block = BlockHash::with_capacity(0);
        block.text("城市路線 α");
        block.n(42_u64);
        block.json(&json!({"k": [1, 2, "𐍈"]})).unwrap();
        let mut independent = b"laneflow:urban-observation-leaf:v4\0".to_vec();
        independent.extend_from_slice(&block.bytes);
        assert_eq!(block.digest(), *blake3::hash(&independent).as_bytes());
    }

    fn fixture_artifacts() -> (tempfile::TempDir, Artifacts) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let config = laneflow_urban_generator::UrbanConfig::parse(include_str!(
            "../../../examples/config/cn-urban.toml"
        ))
        .unwrap();
        laneflow_urban_generator::generate(
            &config,
            laneflow_urban_generator::Scale::Fixture,
            &source,
            None,
        )
        .unwrap();
        let artifacts = Artifacts::load(&source).unwrap();
        (temp, artifacts)
    }

    fn execution(workers: u32) -> laneflow_runtime::ExecutionConfig {
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap())
    }

    fn fixture_plan(artifacts: &Artifacts) -> crate::ResolvedPlan {
        crate::ResolvedPlan::for_case(
            artifacts,
            crate::UrbanCase::SustainedActive,
            crate::Window::probe(2_048).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn observation_digests_are_worker_count_invariant() {
        let (_temp, artifacts) = fixture_artifacts();
        let plan = fixture_plan(&artifacts);
        let run = |workers| {
            let mut h = Harness::install(&artifacts, &plan, execution(workers)).unwrap();
            (0..4)
                .map(|_| {
                    let record = h.advance().unwrap();
                    (record.state_digest, record.event_digest)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(1), run(4));
    }

    #[test]
    fn run_blocks_partitions_match_across_lengths_and_workers() {
        // 块数 1/2/N、整除与不整除、空输入，串行（pool=None）与 4 线程池两路
        // 结果必须与期望块界逐块一致；块界不重叠且全覆盖 [0, len)。
        const BLOCK: usize = 3;
        let pool = rayon_core::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        for len in [
            0_usize,
            1,
            BLOCK,
            BLOCK + 1,
            2 * BLOCK,
            2 * BLOCK + 1,
            4 * BLOCK,
            4 * BLOCK + 1,
        ] {
            let expected: Vec<(usize, usize)> = (0..len)
                .step_by(BLOCK)
                .map(|from| (from, (from + BLOCK).min(len)))
                .collect();
            let run = |pool: Option<&rayon_core::ThreadPool>| {
                run_blocks(pool, len, BLOCK, |from, to| Ok((from, to)))
                    .into_iter()
                    .map(|outcome| outcome.unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(run(None), expected, "serial len={len}");
            assert_eq!(run(Some(&pool)), expected, "parallel len={len}");
            let covered: Vec<usize> = expected.iter().flat_map(|&(from, to)| from..to).collect();
            assert_eq!(covered, (0..len).collect::<Vec<_>>(), "len={len}");
        }
    }

    #[test]
    fn run_blocks_reports_per_block_errors_in_block_order() {
        // 块函数对特定块返回 Err 时，逐块结果向量保持块序：该位置为 Err、
        // 其余块仍返回 Ok；len=7、block_size=3 时 4 线程池的 workers（4）> 块数（3）。
        let pool = rayon_core::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        for pool in [None, Some(&pool)] {
            let outcomes = run_blocks(pool, 7, 3, |from, _to| {
                if from == 3 {
                    Err(invalid("injected block failure"))
                } else {
                    Ok(from)
                }
            });
            assert_eq!(outcomes.len(), 3);
            assert_eq!(outcomes[0].as_ref().unwrap(), &0);
            assert!(
                outcomes[1]
                    .as_ref()
                    .unwrap_err()
                    .to_string()
                    .contains("injected block failure")
            );
            assert_eq!(outcomes[2].as_ref().unwrap(), &6);
        }
    }

    // 逐拍不变量的违例注入：harness 自有状态（`claims`/`individuals`/`slots`/`births`）
    // 可直接篡改，注入后同一拍 state_impl 即报错。世界派生状态（车身重叠、
    // 等待区容量/成员、front 越界、停车 binding 矩阵）无法经公共 API 伪造——
    // Runtime 自身会拦截这类违例，其防线在 Runtime 自身的测试中覆盖。
    #[test]
    fn state_rejects_claim_without_current_reservation_owner() {
        // 资源声明归属：声明的 id 与个体记录不符（incarnation 改大），
        // 持有方校验先于排他判定逐条执行，同一拍报归属错误。
        let (_temp, artifacts) = fixture_artifacts();
        let plan = fixture_plan(&artifacts);
        let mut h = Harness::install(&artifacts, &plan, execution(1)).unwrap();
        h.advance().unwrap();
        let slot = h
            .individuals
            .iter()
            .position(|individual| individual.handle.is_some())
            .expect("fixture has live individuals");
        let forged = IndividualId {
            incarnation: h.individuals[slot].id.incarnation + 1,
            ..h.individuals[slot].id
        };
        h.claims.insert((forged, 0, 0), 0);
        assert!(
            state_impl(&mut h, 64)
                .unwrap_err()
                .to_string()
                .contains("claim has no current reservation owner")
        );
    }

    #[test]
    fn state_rejects_non_exclusive_committed_conflict_claims() {
        // 资源声明排他：merge 按 map 序逐条先插入 zone_owners 再判定持有方，
        // 首个同 zone 的声明必须先通过持有方校验，排他错误才会在第二条
        // 触发。持有方通过需要世界里真实的冲突预约：SUSTAINED-ACTIVE
        // 等负载场景的冲突裁决全程休眠，UNCONTROLLED-YIELD 的
        // opposing pulse 持预约通过冲突区（实测第 133 拍起，确定性）。
        let (_temp, artifacts) = fixture_artifacts();
        let plan = crate::ResolvedPlan::for_case(
            &artifacts,
            crate::UrbanCase::UncontrolledYield,
            crate::Window::probe(2_048).unwrap(),
        )
        .unwrap();
        let mut h = Harness::install(&artifacts, &plan, execution(1)).unwrap();
        let holder = (0..600)
            .find_map(|_| {
                let holder = h
                    .individuals
                    .iter()
                    .find(|individual| {
                        individual
                            .handle
                            .and_then(|handle| h.world.conflict_reservation(handle))
                            .is_some()
                    })
                    .map(|individual| individual.id);
                if holder.is_some() {
                    return holder;
                }
                h.advance().unwrap();
                None
            })
            .expect("UNCONTROLLED-YIELD fixture produces a conflict reservation holder");
        // 真 owner 的 claim（map 序在前）+ 同 zone 不同 owner 的 claim。
        h.claims.insert((holder, 0, 0), 0);
        h.claims.insert(
            (
                IndividualId {
                    incarnation: holder.incarnation + 1,
                    ..holder
                },
                0,
                0,
            ),
            0,
        );
        assert!(
            state_impl(&mut h, 64)
                .unwrap_err()
                .to_string()
                .contains("committed conflict claims are not exclusive")
        );
    }

    #[test]
    fn state_rejects_live_identity_violation() {
        // live 身份守恒：摘掉一个个体的 handle 并从 slots 删除其条目，
        // world 的 live 数（不变）与观测 live 数（-1）不一致。
        // 必须同时清空 step_before 对应槽位，否则 state_block 的拍间对照
        // 会因 handle 缺失先 panic；只删 slots 条目则先报
        // "handle identity map differs"，到不了守恒检查。
        let (_temp, artifacts) = fixture_artifacts();
        let plan = fixture_plan(&artifacts);
        let mut h = Harness::install(&artifacts, &plan, execution(1)).unwrap();
        h.advance().unwrap();
        let slot = h
            .individuals
            .iter()
            .position(|individual| individual.handle.is_some())
            .expect("fixture has live individuals");
        let handle = h.individuals[slot].handle.take().unwrap();
        h.step_before[slot] = None;
        h.slots.remove(&handle);
        assert!(
            state_impl(&mut h, 64)
                .unwrap_err()
                .to_string()
                .contains("live identity conservation failed")
        );
    }

    #[test]
    fn state_rejects_birth_removal_identity_violation() {
        // 出生/移除守恒：births 凭空 +1，plan 期望 live 数与观测不符。
        let (_temp, artifacts) = fixture_artifacts();
        let plan = fixture_plan(&artifacts);
        let mut h = Harness::install(&artifacts, &plan, execution(1)).unwrap();
        h.advance().unwrap();
        h.births += 1;
        assert!(
            state_impl(&mut h, 64)
                .unwrap_err()
                .to_string()
                .contains("birth/removal identity conservation failed")
        );
    }

    #[test]
    fn state_tree_digest_is_invariant_across_block_worker_partitioning() {
        let (_temp, artifacts) = fixture_artifacts();
        let plan = fixture_plan(&artifacts);
        let mut h = Harness::install(&artifacts, &plan, execution(1)).unwrap();
        h.advance().unwrap();
        h.pool = None;
        let sequential = state_impl(&mut h, 64).unwrap().0;
        h.pool = Some(
            rayon_core::ThreadPoolBuilder::new()
                .num_threads(4)
                .build()
                .unwrap(),
        );
        let parallel = state_impl(&mut h, 64).unwrap().0;
        assert_eq!(sequential, parallel);
    }
}
