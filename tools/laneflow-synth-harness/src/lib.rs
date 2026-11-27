//! LF-SYNTH-v2 语义验证与表现测量；结果不直接构成产品认证。

mod burst;
mod decel;
mod handoff;
pub mod host;
mod install;
pub mod measurement;
mod observe;
mod signal_reference;
mod verification;
pub mod verification_cost;
pub use burst::restore;

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use laneflow_runtime::{ExecutionConfig, TickInput, deterministic_state_digest};
use laneflow_synth_generator::{Bundle, Case, Identity, verify_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use verification_cost::{Noop, Phase, Probe, measured};

/// 读取、消费或语义核验失败；失败不产生 complete=true 回执。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("语义验证失败：{0}")]
    Invalid(String),
    #[error(transparent)]
    Input(#[from] laneflow_synth_generator::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn failure(stage: &str, detail: impl std::fmt::Debug) -> Error {
    Error::Invalid(format!("{stage}: {detail:?}"))
}

/// 当前验收语义协议（`lf-synth-execution.md` 第 1 节的拍初独立信号许可）。
/// 每份语义回执显式携带；消费入口严格校验，缺失、旧值或未知值一律拒绝，
/// 旧报告不得通过缺省值或改标签混入。
pub const SEMANTIC_CONTRACT: &str = "lf-synth-execution-2";

/// 校验回执携带当前语义验收协议。
///
/// # Errors
/// 回执缺少 `semantic_contract` 字段或其值不是 [`SEMANTIC_CONTRACT`] 时失败。
pub fn ensure_semantic_contract(report: &Value) -> Result<(), Error> {
    require(report["semantic_contract"] == SEMANTIC_CONTRACT, || {
        format!(
            "回执语义协议不符：期望 {SEMANTIC_CONTRACT}，实际 {:?}",
            report["semantic_contract"]
        )
    })
}

/// 运行回执（`lf-synth-semantic-run-1` / `lf-synth-burst-run-1`）的统一准入：
/// 当前语义协议，且超 profile 减速样本已全部归因到具名例外（#1487）。
///
/// # Errors
/// 语义协议不符，或归因字段缺失、类别集合不符、类别之和不等于总计数、存在
/// 未归因样本时失败。
pub fn ensure_semantic_receipt(report: &Value) -> Result<(), Error> {
    ensure_semantic_contract(report)?;
    decel::check_receipt(report)
}

fn require(valid: bool, context: impl FnOnce() -> String) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(context()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    format: String,
    identity: Identity,
    warm_up_ticks: u64,
    observation_ticks: u64,
    boundary_order: Vec<String>,
    scheduled: Vec<Value>,
    rules: Vec<Value>,
}

impl Plan {
    fn handoff_checkpoints(&self) -> [u64; 4] {
        [
            0,
            self.warm_up_ticks,
            2 * self.warm_up_ticks,
            self.warm_up_ticks + self.observation_ticks,
        ]
    }

    fn read(bundle: &Bundle) -> Result<Self, Error> {
        let plan: Self = serde_json::from_slice(&bundle.files()["input-plan.json"])?;
        let identity = &bundle.manifest().identity;
        let cycle = cycle_ms(identity.fixed_delta_ms);
        let lane_change_window = identity.lane_change_window();
        let warm = if identity.lane_change().is_some() {
            lane_change_window.0
        } else {
            (4 * cycle / u64::from(identity.fixed_delta_ms)).max(512)
        };
        let observation = if identity.lane_change().is_some() {
            lane_change_window.1
        } else if identity.case == Case::W4 {
            2
        } else {
            (8 * cycle / u64::from(identity.fixed_delta_ms)).max(4_096)
        };
        require(
            plan.identity == *identity
                && plan.format == "lf-synth-input-plan-1"
                && plan.warm_up_ticks == warm
                && plan.observation_ticks == observation
                && (plan.scheduled.is_empty()
                    == matches!(&identity.case, Case::W1 | Case::W2 | Case::LaneChange(_)))
                && !plan.rules.is_empty()
                && !plan.boundary_order.is_empty(),
            || "只接受冻结完整窗口与对应命令计划".into(),
        )?;
        // verify_directory 已把规则与同身份重建字节比较；此处不能绕过该受检入口。
        Ok(plan)
    }
}

fn durations(dt: u32) -> [u64; 6] {
    [30_000_u64, 3_000, 1_000, 20_000, 3_000, 1_000]
        .map(|value| value.div_ceil(u64::from(dt)) * u64::from(dt))
}

fn cycle_ms(dt: u32) -> u64 {
    durations(dt).into_iter().sum()
}

#[derive(Debug, Serialize)]
struct Checkpoint {
    tick: u64,
    time_ms: u64,
    state_digest: String,
    public_trace_digest: String,
    transitions: u64,
    /// L1 在此检查点之前观察到的实际横移阶段。
    #[serde(skip_serializing_if = "Option::is_none")]
    handoff_journal: Option<Vec<(u64, u16, u32, u32)>>,
    public_state_digest: String,
    /// 共享验证会话的检查点材料（W1/W2 接线路径；L 族为 `None`）。只随
    /// 边界证据写入报告，恢复消费由 burst 侧接入后启用（设计 §7）。
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<Value>,
}

fn checkpoint(sample: &install::Sample, digest: &blake3::Hasher) -> Result<Checkpoint, Error> {
    let snapshot = sample
        .world
        .capture_snapshot()
        .map_err(|e| failure("snapshot", e))?;
    let state_digest =
        deterministic_state_digest(&snapshot).map_err(|e| failure("state digest", e))?;
    Ok(Checkpoint {
        tick: sample.world.tick_index(),
        time_ms: sample.world.time_ms(),
        state_digest: state_digest
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        public_trace_digest: digest.clone().finalize().to_hex().to_string(),
        transitions: sample.transitions,
        handoff_journal: sample
            .lane_change_route
            .map(|_| sample.lane_change_journal.clone()),
        public_state_digest: public_state_digest(&sample.world)?,
        verification: verification_material(sample)?,
    })
}

/// 会话检查点材料（W1/W2 与 burst 两条接线路径共用）：有会话时取当前
/// 边界的验证检查点并编码为 JSON，无会话（L 族专项或直装测试）为 `None`。
pub(crate) fn verification_material(sample: &install::Sample) -> Result<Option<Value>, Error> {
    match sample.session.as_ref() {
        Some(session) => {
            let material = session
                .checkpoint(&sample.input_identity)
                .map_err(|e| failure("验证会话检查点", e))?;
            serde_json::to_value(material)
                .map(Some)
                .map_err(|e| failure("验证检查点编码", e))
        }
        None => Ok(None),
    }
}

/// 通过回执的报告编码（W1/W2 与 burst 两条接线路径共用）：规格身份、
/// 窗口、触发覆盖、见证与命令尝试统计分开表达（设计 §3.3）。
pub(crate) fn receipt_json(receipt: &laneflow_traffic_verification::session::PassReceipt) -> Value {
    let mut triggers = serde_json::Map::new();
    for (rule, count) in receipt.trigger_counts() {
        triggers.insert(rule.0.to_string(), json!(count));
    }
    let mut witnesses = serde_json::Map::new();
    for (witness, count) in receipt.witnesses() {
        witnesses.insert(witness.0.to_string(), json!(count));
    }
    json!({
        "spec": receipt.spec_name(),
        "domain": receipt.domain(),
        "semantic_version": receipt.semantic_version(),
        "successful_steps": receipt.successful_steps(),
        "last_tick": receipt.last_tick(),
        "trigger_counts": triggers,
        "witnesses": witnesses,
        "blocked_attempts": receipt.blocked_attempts(),
        "failed_attempts": receipt.failed_attempts(),
    })
}

/// 公开观测面状态摘要（#1233 F2）：逐 live 车辆公开状态确定性哈希；
/// 快照拒绝时仍可用于检查点等价比对。
pub(crate) fn public_state_digest(world: &laneflow_runtime::TrafficWorld) -> Result<String, Error> {
    let mut hasher = blake3::Hasher::new();
    for handle in world.live_vehicles() {
        let state = world
            .vehicle(*handle)
            .ok_or_else(|| failure("public digest", handle))?;
        let status = match state.status() {
            laneflow_runtime::VehicleStatus::Active => 0_u32,
            laneflow_runtime::VehicleStatus::Parked => 1,
            laneflow_runtime::VehicleStatus::Completed => 2,
        };
        hasher.update(format!("{handle:?}|{:?}", state.route()).as_bytes());
        hasher.update(&state.route_edge_index().to_le_bytes());
        hasher.update(&state.progress_mm().to_le_bytes());
        hasher.update(&state.carry_um().to_le_bytes());
        hasher.update(&state.speed_mm_s().to_le_bytes());
        hasher.update(&status.to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// 从 manifest 身份 JSON 提取场景族标签（内部命名口径，#1233）。
pub fn scenario_label(identity: &serde_json::Value) -> &'static str {
    serde_json::from_value::<laneflow_synth_generator::Case>(identity["case"].clone())
        .map(|case| case.label())
        .unwrap_or("")
}

/// 消费一个正式输入目录，在全新目录保存逐拍轨迹与完整回执。
///
/// # Errors
/// 输入不符合生成合同、提交格式非法、输出已存在、I/O、执行配置不受支持、安装/步进或逐拍
/// 语义核验失败时返回错误。失败目录保留 complete=false 和已写轨迹，不能当成完成结果。
pub fn run(
    input: &Path,
    implementation_commit: &str,
    execution: ExecutionConfig,
    output: &Path,
) -> Result<Value, Error> {
    run_with_probe(input, implementation_commit, execution, output, &mut Noop)
}

fn run_with_probe<P: Probe>(
    input: &Path,
    implementation_commit: &str,
    execution: ExecutionConfig,
    output: &Path,
    probe: &mut P,
) -> Result<Value, Error> {
    require(
        implementation_commit.len() == 40
            && implementation_commit
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        || "实现提交须为完整小写 Git SHA".into(),
    )?;
    fs::create_dir(output)?;
    probe.output_created(output)?;
    let mut status = json!({"format":"lf-synth-semantic-run-1","complete":false,
        "semantic_contract":SEMANTIC_CONTRACT,
        "implementation_commit":implementation_commit,"workers":execution.worker_count().get()});
    write_json(&output.join("report.json"), &status)?;
    let result = run_inner(input, implementation_commit, execution, output, probe);
    match result {
        Ok(report) => {
            write_json(&output.join("report.json"), &report)?;
            Ok(report)
        }
        Err(error) => {
            status["error"] = Value::String(error.to_string());
            write_json(&output.join("report.json"), &status)?;
            Err(error)
        }
    }
}

fn run_inner<P: Probe>(
    input: &Path,
    implementation_commit: &str,
    execution: ExecutionConfig,
    output: &Path,
    probe: &mut P,
) -> Result<Value, Error> {
    let bundle = verify_directory(input)?;
    let plan = Plan::read(&bundle)?;
    run_plan(
        &bundle,
        &plan,
        implementation_commit,
        execution,
        output,
        probe,
    )
}

fn run_plan<P: Probe>(
    bundle: &Bundle,
    plan: &Plan,
    implementation_commit: &str,
    execution: ExecutionConfig,
    output: &Path,
    probe: &mut P,
) -> Result<Value, Error> {
    probe.bind(bundle, plan)?;
    if matches!(&plan.identity.case, Case::W3 | Case::W4) {
        return burst::run_with_probe(
            bundle,
            plan,
            implementation_commit,
            execution,
            output,
            probe,
        );
    }
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let mut sample = measured(probe, Phase::Setup, |_| {
        let mut sample = install::Sample::new(bundle, execution)?;
        if plan.identity.lane_change().is_none() {
            // W1/W2 正式 run 接线共享验证会话；L 族专项组合独立（设计 §10）。
            sample.session = Some(verification::start_session(total)?);
            sample.input_identity = verification::input_identity(bundle);
            sample
                .session
                .as_mut()
                .expect("刚装入会话")
                .confirm_initial_boundary(0)
                .map_err(|e| failure("验证会话初始边界", e))?;
        }
        Ok(sample)
    })?;
    verification_cost::sample_buffers(probe, &sample);
    let setup = probe.start();
    let mut trace = BufWriter::new(File::create(output.join("ticks.bin"))?);
    trace.write_all(b"lf-synth-public-ticks-1\0")?;
    let mut stream = blake3::Hasher::new();
    stream.update(b"lf-synth-public-stream-1\0");
    let boundaries: Vec<u64> = if plan.identity.lane_change().is_some() {
        plan.handoff_checkpoints().to_vec()
    } else {
        vec![
            0,
            plan.warm_up_ticks,
            plan.warm_up_ticks + 1_024,
            plan.warm_up_ticks + 2_048,
            plan.warm_up_ticks + 4_096,
            total,
        ]
    };
    let mut checkpoints = Vec::new();
    let ranks = sample
        .handles
        .iter()
        .copied()
        .enumerate()
        .map(|(rank, handle)| (handle, rank))
        .collect();
    let mut events = if plan.identity.lane_change().is_some() {
        measurement::events::Recorder::new_lane_change(total + 1, plan.identity.scale)
    } else {
        measurement::events::Recorder::new(total + 1, plan.identity.scale)
    };
    probe.end(Phase::Setup, setup);
    for tick in 0..=total {
        probe.boundary(tick);
        let outcome = if tick == 0 {
            None
        } else {
            measured(probe, Phase::Session, |_| {
                if let Some(session) = sample.session.as_mut() {
                    session
                        .declare_commands(0)
                        .and_then(|()| session.advance_to_pre_tick())
                        .and_then(|()| session.begin_step())
                        .map_err(|e| failure("验证会话命令阶段", e))?;
                }
                Ok(())
            })?;
            probe.runtime_step_tick(tick);
            let result = measured(probe, Phase::RuntimeStep, |_| {
                sample.step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
            })?;
            require(
                result.tick_index() == tick
                    && result.time_ms() == tick * u64::from(plan.identity.fixed_delta_ms),
                || format!("step time at {tick}"),
            )?;
            Some(result)
        };
        let digest = measured(probe, Phase::Observation, |probe| {
            sample.observe_with_probe(
                tick,
                outcome
                    .as_ref()
                    .is_none_or(|o| o.parking_arrivals().is_empty()),
                probe,
            )
        })?;
        measured(probe, Phase::Session, |_| {
            if tick > 0 {
                // 步进结果在观测消费完本拍检查后恰核销一次（设计 §6）。
                if let Some(session) = sample.session.as_mut() {
                    session
                        .note_successful_step(tick)
                        .map_err(|e| failure("验证会话步进协议", e))?;
                }
            }
            Ok(())
        })?;
        verification_cost::sample_buffers(probe, &sample);
        measured(probe, Phase::Evidence, |_| {
            events.record(&sample.world, &ranks, &sample.routes, outcome.as_ref())?;
            trace.write_all(&tick.to_le_bytes())?;
            trace.write_all(digest.as_bytes())?;
            stream.update(&tick.to_le_bytes());
            stream.update(digest.as_bytes());
            if boundaries.contains(&tick) {
                checkpoints.push(checkpoint(&sample, &stream)?);
            }
            if tick % 1_024 == 0 {
                trace.flush()?;
                eprintln!(
                    "{:?} scale={} dt={} tick={tick}/{total}",
                    plan.identity.case, plan.identity.scale, plan.identity.fixed_delta_ms
                );
            }
            Ok(())
        })?;
        probe.boundary_complete();
    }
    trace.flush()?;
    events.write(&output.join("events.bin"))?;
    let finish = probe.start();
    if plan.identity.lane_change().is_some() {
        require(
            sample
                .lane_change_last_kind
                .iter()
                .all(|&phase| phase == 12),
            || "窗口内两车必须完成横移且车尾清空".into(),
        )?;
        require(
            sample.crossed.iter().enumerate().all(|(rank, &crossed)| {
                crossed
                    == sample
                        .lane_change_routes
                        .contains(&(sample.state.vehicles[rank].route_rank as usize))
            }),
            || "窗口内全体换道者前杠交接且背景不发生 0→1".into(),
        )?;
    } else {
        require(
            sample
                .crossed
                .iter()
                .zip(&sample.initial)
                .all(|(&crossed, initial)| {
                    crossed == (initial.status() == laneflow_runtime::VehicleStatus::Active)
                }),
            || "窗口内每辆 Active 必须恰好完成 entry→exit；Parked 不发生转换".into(),
        )?;
    }
    // 会话终结：完整窗口、覆盖、必需见证与无违反四条件成立才产生回执；
    // 任一不满足如实失败，不以未测量推断通过（设计 §3.3）。
    let verification_receipt = match sample.session.as_mut() {
        Some(session) => {
            let receipt = session
                .finish()
                .map_err(|e| crate::Error::Invalid(format!("验证会话终结被拒：{e:?}")))?;
            Some(receipt_json(&receipt))
        }
        None => None,
    };
    probe.end(Phase::Finish, finish);
    let deceleration = sample.deceleration.receipt()?;
    Ok(json!({"format":"lf-synth-semantic-run-1","complete":true,
        "semantic_contract":SEMANTIC_CONTRACT,
        "scope":if plan.identity.lane_change().is_some() {
            "L1_lane_change_competition_success_path_not_performance_or_product_certification"
        } else {
            "W1_W2_semantics_only_not_performance_or_product_certification"
        },
        "implementation_commit":implementation_commit,"input_manifest":bundle.manifest(),
        "workers":sample.world.execution_config().worker_count().get(),
        "warm_up_ticks":plan.warm_up_ticks,"observation_ticks":plan.observation_ticks,
        "validated_boundaries":total+1,"caller_commands_after_initialization":0,
        "active":bundle.manifest().counts["active"],"parked":bundle.manifest().counts["parked"],
        "gate_crossings":sample.transitions,"completion_count":0,"parking_arrival_count":0,
        "max_observed_deceleration_mm_s2":deceleration["max_observed_deceleration_mm_s2"],
        "deceleration_above_profile_count":deceleration["deceleration_above_profile_count"],
        "deceleration_above_profile_attribution":deceleration["deceleration_above_profile_attribution"],
        "public_trace_digest":stream.finalize().to_hex().to_string(),"checkpoints":checkpoints,
        "verification_receipt":verification_receipt,
        "ordered_events_digest":events.digest()}))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
#[path = "handoff/presentation_tests.rs"]
mod handoff_presentation_tests;
#[cfg(test)]
mod tests;
