//! LF-SYNTH-v2 语义验证与表现测量；结果不直接构成产品认证。

mod burst;
mod install;
pub mod measurement;
mod observe;
pub use burst::restore;

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use laneflow_runtime::{ExecutionConfig, TickInput, deterministic_state_digest};
use laneflow_synth_generator::{Bundle, Case, Identity, verify_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    fn read(bundle: &Bundle) -> Result<Self, Error> {
        let plan: Self = serde_json::from_slice(&bundle.files()["input-plan.json"])?;
        let identity = &bundle.manifest().identity;
        let cycle = cycle_ms(identity.fixed_delta_ms);
        let warm = (4 * cycle / u64::from(identity.fixed_delta_ms)).max(512);
        let observation = if identity.case == Case::W4 {
            2
        } else {
            (8 * cycle / u64::from(identity.fixed_delta_ms)).max(4_096)
        };
        require(
            plan.identity == *identity
                && plan.format == "lf-synth-input-plan-1"
                && plan.warm_up_ticks == warm
                && plan.observation_ticks == observation
                && (plan.scheduled.is_empty() == matches!(identity.case, Case::W1 | Case::W2))
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
    })
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
    require(
        implementation_commit.len() == 40
            && implementation_commit
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        || "实现提交须为完整小写 Git SHA".into(),
    )?;
    fs::create_dir(output)?;
    let mut status = json!({"format":"lf-synth-semantic-run-1","complete":false,
        "implementation_commit":implementation_commit,"workers":execution.worker_count().get()});
    write_json(&output.join("report.json"), &status)?;
    let result = run_inner(input, implementation_commit, execution, output);
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

fn run_inner(
    input: &Path,
    implementation_commit: &str,
    execution: ExecutionConfig,
    output: &Path,
) -> Result<Value, Error> {
    let bundle = verify_directory(input)?;
    let plan = Plan::read(&bundle)?;
    if matches!(plan.identity.case, Case::W3 | Case::W4) {
        return burst::run(&bundle, &plan, implementation_commit, execution, output);
    }
    let mut sample = install::Sample::new(&bundle, execution)?;
    let mut trace = BufWriter::new(File::create(output.join("ticks.bin"))?);
    trace.write_all(b"lf-synth-public-ticks-1\0")?;
    let mut stream = blake3::Hasher::new();
    stream.update(b"lf-synth-public-stream-1\0");
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let boundaries = [
        0,
        plan.warm_up_ticks,
        plan.warm_up_ticks + 1_024,
        plan.warm_up_ticks + 2_048,
        plan.warm_up_ticks + 4_096,
        total,
    ];
    let mut checkpoints = Vec::new();
    let ranks = sample
        .handles
        .iter()
        .copied()
        .enumerate()
        .map(|(rank, handle)| (handle, rank))
        .collect();
    let mut events = measurement::events::Recorder::new(total + 1, plan.identity.scale);
    for tick in 0..=total {
        let outcome = if tick == 0 {
            None
        } else {
            let result = sample
                .world
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure(&format!("step {tick}"), e))?;
            require(
                result.tick_index() == tick
                    && result.time_ms() == tick * u64::from(plan.identity.fixed_delta_ms),
                || format!("step time at {tick}"),
            )?;
            Some(result)
        };
        let digest = sample.observe(
            tick,
            outcome
                .as_ref()
                .is_none_or(|o| o.parking_arrivals().is_empty()),
        )?;
        events.record(&sample.world, &ranks, outcome.as_ref())?;
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
    }
    trace.flush()?;
    events.write(&output.join("events.bin"))?;
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
    Ok(json!({"format":"lf-synth-semantic-run-1","complete":true,
        "scope":"W1_W2_semantics_only_not_performance_or_product_certification",
        "implementation_commit":implementation_commit,"input_manifest":bundle.manifest(),
        "workers":sample.world.execution_config().worker_count().get(),
        "warm_up_ticks":plan.warm_up_ticks,"observation_ticks":plan.observation_ticks,
        "validated_boundaries":total+1,"caller_commands_after_initialization":0,
        "active":bundle.manifest().counts["active"],"parked":bundle.manifest().counts["parked"],
        "gate_crossings":sample.transitions,"completion_count":0,"parking_arrival_count":0,
        "max_observed_deceleration_mm_s2":sample.max_deceleration_mm_s2,
        "deceleration_above_profile_count":sample.deceleration_above_profile_count,
        "public_trace_digest":stream.finalize().to_hex().to_string(),"checkpoints":checkpoints,
        "ordered_events_digest":events.digest()}))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests;
