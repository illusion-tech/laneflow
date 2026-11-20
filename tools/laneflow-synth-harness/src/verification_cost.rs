//! 完整 W 族语义路径的验证成本计量（traffic-verification §9）。计数构建与
//! 时间构建分开；计量材料不进入原语义报告、轨迹或通过回执。

use std::path::Path;
use std::time::Instant;

use laneflow_runtime::ExecutionConfig;
use laneflow_synth_generator::{Bundle, Case};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Plan, require, write_json};

const FILE: &str = "verification-cost.json";
const FORMAT: &str = "lf-synth-verification-cost-1";

/// 时间与结构/分配计数必须使用各自的独立构建和进程。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 不含 allocator instrumentation 的完整语义运行时间。
    Timing,
    /// 独立分配构建的扫描、片段与容量计数；不输出时间证据。
    Counts,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Timing => "timing",
            Self::Counts => "counts",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Setup,
    Observation,
    Session,
    Commands,
    RuntimeStep,
    Evidence,
    Finish,
}

impl Phase {
    const ALL: [Self; 7] = [
        Self::Setup,
        Self::Observation,
        Self::Session,
        Self::Commands,
        Self::RuntimeStep,
        Self::Evidence,
        Self::Finish,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Observation => "observation",
            Self::Session => "session",
            Self::Commands => "commands_and_receipts",
            Self::RuntimeStep => "runtime_step",
            Self::Evidence => "evidence",
            Self::Finish => "finish",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Scan {
    LiveOrder,
    SignalCount,
    Signals,
    Vehicles,
    BodyOrder,
    EventCandidates,
    TransitionEvents,
    ParkingArrivals,
    W4Changed,
    ChangedReset,
}

impl Scan {
    #[cfg(feature = "verification-cost-counts")]
    const ALL: [Self; 10] = [
        Self::LiveOrder,
        Self::SignalCount,
        Self::Signals,
        Self::Vehicles,
        Self::BodyOrder,
        Self::EventCandidates,
        Self::TransitionEvents,
        Self::ParkingArrivals,
        Self::W4Changed,
        Self::ChangedReset,
    ];

    #[cfg(feature = "verification-cost-counts")]
    fn name(self) -> &'static str {
        match self {
            Self::LiveOrder => "live_order",
            Self::SignalCount => "signal_count",
            Self::Signals => "signal_aspects",
            Self::Vehicles => "vehicle_state",
            Self::BodyOrder => "body_order",
            Self::EventCandidates => "event_candidates",
            Self::TransitionEvents => "transition_events",
            Self::ParkingArrivals => "parking_arrivals",
            Self::W4Changed => "w4_changed",
            Self::ChangedReset => "changed_reset",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Buffer {
    Previous,
    Crossed,
    Trace,
    LaneOrder,
    Changed,
    Pending,
}

impl Buffer {
    #[cfg(feature = "verification-cost-counts")]
    const ALL: [Self; 6] = [
        Self::Previous,
        Self::Crossed,
        Self::Trace,
        Self::LaneOrder,
        Self::Changed,
        Self::Pending,
    ];

    #[cfg(feature = "verification-cost-counts")]
    fn name(self) -> &'static str {
        match self {
            Self::Previous => "sample.previous",
            Self::Crossed => "sample.crossed",
            Self::Trace => "sample.trace_bytes",
            Self::LaneOrder => "burst.lane_order",
            Self::Changed => "burst.changed",
            Self::Pending => "burst.pending",
        }
    }
}

/// 普通入口以 Noop 单态化；计数钩子只写当前驱动器的标量，不在逐车路径
/// 分配、读时钟或使用共享原子。扫描次数表示遍历入口，visits 表示逻辑行。
pub(crate) trait Probe {
    type Mark;

    fn start(&mut self) -> Self::Mark;
    fn end(&mut self, phase: Phase, mark: Self::Mark);

    #[inline]
    fn output_created(&mut self, _output: &Path) -> Result<(), Error> {
        Ok(())
    }
    #[inline]
    fn bind(&mut self, _bundle: &Bundle, _plan: &Plan) -> Result<(), Error> {
        Ok(())
    }
    #[inline]
    fn boundary(&mut self, _tick: u64) {}
    #[inline]
    fn runtime_step_tick(&mut self, _tick: u64) {}
    #[inline]
    fn command_boundary(&mut self, _tick: u64) {}
    #[inline]
    fn boundary_complete(&mut self) {}
    #[inline]
    fn scan(&mut self, _scan: Scan) {}
    #[inline]
    fn visit(&mut self, _scan: Scan, _rows: usize) {}
    #[inline]
    fn body_fragment(&mut self) {}
    #[inline]
    fn buffer(&mut self, _buffer: Buffer, _length: usize, _capacity: usize, _item_bytes: usize) {}
}

pub(crate) struct Noop;

impl Probe for Noop {
    type Mark = ();

    #[inline]
    fn start(&mut self) {}
    #[inline]
    fn end(&mut self, _phase: Phase, _mark: ()) {}
}

pub(crate) fn measured<P: Probe, T>(
    probe: &mut P,
    phase: Phase,
    operation: impl FnOnce(&mut P) -> Result<T, Error>,
) -> Result<T, Error> {
    let mark = probe.start();
    let result = operation(probe);
    probe.end(phase, mark);
    result
}

pub(crate) fn sample_buffers(probe: &mut impl Probe, sample: &crate::install::Sample) {
    for (buffer, length, capacity, item_bytes) in [
        (
            Buffer::Previous,
            sample.previous.len(),
            sample.previous.capacity(),
            size_of::<laneflow_runtime::VehicleState>(),
        ),
        (
            Buffer::Crossed,
            sample.crossed.len(),
            sample.crossed.capacity(),
            size_of::<bool>(),
        ),
        (
            Buffer::Trace,
            sample.trace_bytes.len(),
            sample.trace_bytes.capacity(),
            size_of::<u8>(),
        ),
    ] {
        probe.buffer(buffer, length, capacity, item_bytes);
    }
}

struct Common {
    mode: Mode,
    implementation: String,
    workers: u32,
    manifest: Value,
    input_identity: String,
    warm: u64,
    end: u64,
    case: Option<Case>,
    completed_boundaries: u64,
    created: bool,
}

impl Common {
    fn new(mode: Mode, implementation: &str, execution: ExecutionConfig) -> Self {
        Self {
            mode,
            implementation: implementation.to_owned(),
            workers: execution.worker_count().get(),
            manifest: Value::Null,
            input_identity: String::new(),
            warm: 0,
            end: 0,
            case: None,
            completed_boundaries: 0,
            created: false,
        }
    }

    fn bind(&mut self, bundle: &Bundle, plan: &Plan) -> Result<(), Error> {
        require(plan.identity.lane_change().is_none(), || {
            "验证成本入口仅接受 W1–W4 完整计划".into()
        })?;
        self.manifest = serde_json::to_value(bundle.manifest())?;
        // 本变体早于会话接线模块；输入身份按同一公式内联（配置摘要@来源提交）。
        self.input_identity = format!(
            "{}@{}",
            bundle.manifest().config_digest,
            bundle.manifest().source_commit
        );
        self.warm = plan.warm_up_ticks;
        self.end = plan.warm_up_ticks + plan.observation_ticks;
        self.case = Some(plan.identity.case.clone());
        Ok(())
    }

    fn output_created(&mut self, output: &Path) -> Result<(), Error> {
        self.created = true;
        write_json(&output.join(FILE), &self.report(false))
    }

    fn report(&self, complete: bool) -> Value {
        json!({
            "format":FORMAT,"complete":complete,"harness":"synth","mode":self.mode.name(),
            "purpose":"verification-cost",
            "implementation_commit":self.implementation,"input_identity":self.input_identity,
            "input_manifest":self.manifest,"semantic_contract":crate::SEMANTIC_CONTRACT,
            "workers":self.workers,"window":{"warm_up_ticks":self.warm,"end_tick":self.end,
                "observation_ticks":self.end-self.warm,"initial_boundary_included":true},
            "completed_boundaries":self.completed_boundaries,"semantic_run":"report.json",
            "whole_run_wall_ns":null,"observation_ns":null,"workload_counts":null,
            "allocation":null,"scratch_high_water":null,
            "scope":{
                "variant":"A1_pre_session_wiring_session_phase_not_present_totals_and_rows_are_structural_zero",
                "observation":"fused_fact_reads_checks_encoding_and_boundary_hash_no_session_notes_on_this_variant",
                "session":"not_present_on_this_variant_pre_dates_session_wiring_always_zero",
                "commands_and_receipts":"boundary_planning_runtime_commands_checks_receipts_reordering_and_batch_evidence",
                "finish":"host_checks_and_report_encoding_no_session_finish_on_this_variant",
                "whole_run_wall":"after_initial_cost_file_through_semantic_report_write_includes_meter_binding_and_sample_maintenance_excludes_final_cost_serialization",
                "scans":"observer_logical_visits_not_cpu_loads_excludes_runtime_snapshot_and_evidence_recorder_internals",
                "incomplete_counts":"failed_runs_may_include_visits_from_the_unfinished_last_boundary; completed_boundaries_counts_only_finished_boundaries",
                "scratch":"named_harness_vec_owned_capacity_only_not_total_heap_or_runtime_memory",
                "boundary_samples":"tick_and_observation_scope_identify_observed_state; runtime_step_tick_and_command_boundary_identify_other_phases"
            }
        })
    }

    fn scope(&self, tick: u64) -> usize {
        if tick == 0 {
            0
        } else if tick <= self.warm {
            1
        } else {
            2
        }
    }
}

const SCOPES: [&str; 3] = ["initial", "warm_up", "observation"];

#[derive(Serialize)]
struct TimingRow {
    tick: u64,
    observation_scope: &'static str,
    runtime_step_tick: Option<u64>,
    command_boundary: Option<u64>,
    complete: bool,
    observation_ns: u64,
    session_ns: u64,
    commands_and_receipts_ns: u64,
    runtime_step_ns: u64,
    evidence_ns: u64,
}

struct Timing {
    common: Common,
    totals: [u64; 7],
    rows: Vec<TimingRow>,
    current: Option<usize>,
    started: Option<Instant>,
    elapsed: Option<u64>,
}

impl Timing {
    fn new(implementation: &str, execution: ExecutionConfig) -> Self {
        Self {
            common: Common::new(Mode::Timing, implementation, execution),
            totals: [0; 7],
            rows: Vec::new(),
            current: None,
            started: None,
            elapsed: None,
        }
    }
}

impl Probe for Timing {
    type Mark = Instant;

    fn start(&mut self) -> Instant {
        Instant::now()
    }

    fn end(&mut self, phase: Phase, mark: Instant) {
        let elapsed = nanos(mark);
        self.totals[phase as usize] += elapsed;
        if let Some(index) = self.current {
            let row = &mut self.rows[index];
            match phase {
                Phase::Observation => row.observation_ns += elapsed,
                Phase::Session => row.session_ns += elapsed,
                Phase::Commands => row.commands_and_receipts_ns += elapsed,
                Phase::RuntimeStep => row.runtime_step_ns += elapsed,
                Phase::Evidence => row.evidence_ns += elapsed,
                Phase::Setup | Phase::Finish => {}
            }
        }
    }

    fn output_created(&mut self, output: &Path) -> Result<(), Error> {
        self.common.output_created(output)?;
        self.started = Some(Instant::now());
        Ok(())
    }

    fn bind(&mut self, bundle: &Bundle, plan: &Plan) -> Result<(), Error> {
        self.common.bind(bundle, plan)?;
        let boundaries = usize::try_from(self.common.end + 1)
            .map_err(|_| Error::Invalid("验证计时窗口超出地址空间".into()))?;
        self.rows
            .try_reserve_exact(boundaries)
            .map_err(|error| Error::Invalid(format!("验证计时样本分配失败：{error}")))
    }

    fn boundary(&mut self, tick: u64) {
        self.current = Some(self.rows.len());
        self.rows.push(TimingRow {
            tick,
            observation_scope: SCOPES[self.common.scope(tick)],
            runtime_step_tick: None,
            command_boundary: None,
            complete: false,
            observation_ns: 0,
            session_ns: 0,
            commands_and_receipts_ns: 0,
            runtime_step_ns: 0,
            evidence_ns: 0,
        });
    }

    fn runtime_step_tick(&mut self, tick: u64) {
        if let Some(index) = self.current {
            self.rows[index].runtime_step_tick = Some(tick);
        }
    }

    fn command_boundary(&mut self, tick: u64) {
        if let Some(index) = self.current {
            self.rows[index].command_boundary = Some(tick);
        }
    }

    fn boundary_complete(&mut self) {
        self.common.completed_boundaries += 1;
        if let Some(index) = self.current.take() {
            self.rows[index].complete = true;
        }
    }
}

fn nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn distribution(mut values: Vec<u64>, raw_only: bool) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let raw = raw_only.then(|| values.clone());
    values.sort_unstable();
    let at = |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
    json!({"samples":values.len(),"max_ns":values.last(),"raw_ns":raw,
        "p50_ns":(!raw_only).then(|| at(50)),"p95_ns":(!raw_only).then(|| at(95)),
        "p99_ns":(!raw_only).then(|| at(99))})
}

trait ReportProbe: Probe {
    fn common(&self) -> &Common;
    fn stop(&mut self);
    fn report(&self, complete: bool) -> Result<Value, Error>;
}

impl ReportProbe for Timing {
    fn common(&self) -> &Common {
        &self.common
    }

    fn stop(&mut self) {
        self.elapsed = self.started.map(nanos);
    }

    fn report(&self, complete: bool) -> Result<Value, Error> {
        let mut value = self.common.report(complete);
        value["whole_run_wall_ns"] = json!(self.elapsed);
        value["timing_by_phase_ns"] = Phase::ALL
            .into_iter()
            .map(|phase| (phase.name().to_owned(), json!(self.totals[phase as usize])))
            .collect::<serde_json::Map<_, _>>()
            .into();
        value["observation_ns"] = SCOPES
            .into_iter()
            .map(|scope| {
                let samples = self
                    .rows
                    .iter()
                    .filter(|row| row.observation_scope == scope && row.complete)
                    .map(|row| row.observation_ns)
                    .collect();
                let raw_only = scope == "initial"
                    || (scope == "observation" && self.common.case == Some(Case::W4));
                (scope.to_owned(), distribution(samples, raw_only))
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        value["boundary_samples"] = serde_json::to_value(&self.rows)?;
        Ok(value)
    }
}

#[cfg(feature = "verification-cost-counts")]
mod counts;

fn metered<P: ReportProbe>(
    input: &Path,
    implementation: &str,
    execution: ExecutionConfig,
    output: &Path,
    mut probe: P,
) -> Result<Value, Error> {
    let result = crate::run_with_probe(input, implementation, execution, output, &mut probe);
    probe.stop();
    if !probe.common().created {
        return result.map(|_| Value::Null);
    }
    let mut cost = probe.report(result.is_ok())?;
    if let Err(error) = &result {
        cost["error"] = json!(error.to_string());
    }
    write_json(&output.join(FILE), &cost)?;
    result?;
    Ok(cost)
}

/// 用与普通 `run` 相同的完整 W1–W4 路径输出独立验证成本材料。
/// 普通语义产物保持原格式；本入口不接受短窗口、规则开关或产品性能通过标记。
///
/// # Errors
/// 模式与构建不符、计数构建未装配全局 allocator、输入/执行/语义验证失败、
/// 提交格式错误、输出目录已存在或 I/O 失败时返回错误。已创建目录保留不完整材料。
pub fn run(
    input: &Path,
    implementation: &str,
    execution: ExecutionConfig,
    mode: Mode,
    output: &Path,
) -> Result<Value, Error> {
    match mode {
        Mode::Timing => {
            require(
                !cfg!(feature = "allocation") && !cfg!(feature = "verification-cost-counts"),
                || "timing 必须使用不含 allocation/verification-cost-counts 的独立构建".into(),
            )?;
            metered(
                input,
                implementation,
                execution,
                output,
                Timing::new(implementation, execution),
            )
        }
        Mode::Counts => {
            #[cfg(feature = "verification-cost-counts")]
            {
                require(!cfg!(feature = "allocation"), || {
                    "counts 仅使用 verification-cost-counts，不混入产品 allocation 插桩".into()
                })?;
                counts::require_allocator()?;
                metered(
                    input,
                    implementation,
                    execution,
                    output,
                    counts::Counts::new(implementation, execution),
                )
            }
            #[cfg(not(feature = "verification-cost-counts"))]
            Err(Error::Invalid(
                "counts 需要 verification-cost-counts 独立构建".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests;
