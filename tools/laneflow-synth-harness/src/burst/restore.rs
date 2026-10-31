//! #1197：真实 LFRS 恢复、宿主阶段接续与完整后缀核验；不认证恢复性能。

mod state;

use std::fs;
use std::num::NonZeroU32;
use std::time::Instant;

use laneflow_synth_generator::verify_directory;
use sha2::{Digest, Sha256};

use super::*;
use crate::measurement::events::Recorder;

const TRACE_HEADER: &[u8] = b"lf-synth-restore-ticks-1\0";
const EVENT_HEADER: &[u8] = b"lf-synth-ordered-events-1\0";
const COMMAND_HEADER: &[u8] = b"lf-synth-command-receipts-1\0";

enum Driver {
    Simple(Box<Sample>),
    Burst(Box<Burst>),
}

impl Driver {
    fn sample(&self) -> &Sample {
        match self {
            Self::Simple(s) => s,
            Self::Burst(b) => &b.sample,
        }
    }
    fn sample_mut(&mut self) -> &mut Sample {
        match self {
            Self::Simple(s) => s,
            Self::Burst(b) => &mut b.sample,
        }
    }
    fn observe(
        &mut self,
        plan: &Plan,
        outcome: Option<&StepOutcome>,
    ) -> Result<blake3::Hash, Error> {
        let tick = self.sample().world.tick_index();
        match self {
            Self::Simple(s) => s.observe(
                tick,
                outcome.is_none_or(|o| o.parking_arrivals().is_empty()),
            ),
            Self::Burst(b) => b.observe(tick, plan.warm_up_ticks, outcome),
        }
    }
    fn commands(
        &mut self,
        plan: &Plan,
        scheduled: &[Scheduled],
        records: &mut Records,
    ) -> Result<(), Error> {
        let Self::Burst(b) = self else {
            return Ok(());
        };
        let tick = b.sample.world.tick_index();
        let commands =
            b.boundary_commands(tick, plan.warm_up_ticks + plan.observation_ticks, scheduled)?;
        if commands.is_empty() {
            return Ok(());
        }
        let before = state_digest(&b.sample.world)?;
        let cursor = b.sample.cursor;
        let first = records.journal.count;
        for command in &commands {
            let rank = command.logical_rank as usize;
            let incarnation = *b
                .incarnations
                .get(rank)
                .ok_or_else(|| failure("command rank", rank))?;
            let before_cursor = b.sample.cursor;
            let committed = b.command(command)?;
            records.journal.append(
                command,
                tick,
                [before_cursor, b.sample.cursor],
                [incarnation, b.incarnations[rank]],
                committed,
            )?;
            if !committed {
                b.pending.push(command.clone());
            }
        }
        b.refresh_order();
        records.batches.push(json!({"boundary":tick,"command_count":commands.len(),"first_receipt":first,"input_digest":input_digest(&commands),"pre_command_state_digest":before,"post_command_state_digest":state_digest(&b.sample.world)?,"cursor_before":cursor,"cursor_after":b.sample.cursor}));
        records.journal.output.flush()?;
        Ok(())
    }
    fn finish(&self, plan: &Plan) -> Result<(), Error> {
        match self {
            Self::Burst(b) => b.finish(plan),
            Self::Simple(s) => require(
                s.crossed
                    .iter()
                    .zip(&s.state.vehicles)
                    .all(|(&crossed, v)| crossed == (v.status == "active")),
                || "完整窗口过门次数不符".into(),
            ),
        }
    }
    fn final_host(&self) -> Value {
        let s = self.sample();
        let burst = match self {
            Self::Simple(_) => json!(null),
            Self::Burst(b) => {
                json!({"incarnations":b.incarnations,"total_crossings":b.total_crossings,"arrivals":b.arrivals,"completions":b.completions,"arrival_records":b.arrival_records,"pending":b.pending,"parked_boundary":b.parked_boundary,"left":b.left,"live_ranks":b.live_ranks})
            }
        };
        json!({"crossed":s.crossed,"transitions":s.transitions,"max_deceleration":s.max_deceleration_mm_s2,"excessive_decelerations":s.deceleration_above_profile_count,"burst":burst})
    }
}

struct Records {
    trace: BufWriter<File>,
    trace_count: u64,
    events: Recorder,
    journal: Journal,
    batches: Vec<Value>,
    checkpoints: Vec<Value>,
    simple_ranks: HashMap<VehicleHandle, usize>,
}

impl Records {
    fn new(output: &Path, total: u64, scale: u32, start: u64) -> Result<Self, Error> {
        let mut trace = BufWriter::new(File::create(output.join("ticks.bin"))?);
        trace.write_all(TRACE_HEADER)?;
        Ok(Self {
            trace,
            trace_count: 0,
            events: Recorder::from_tick(total - start + 1, scale, start),
            journal: Journal::new(&output.join("commands.bin"))?,
            batches: Vec::new(),
            checkpoints: Vec::new(),
            simple_ranks: HashMap::new(),
        })
    }
    fn observe(
        &mut self,
        driver: &mut Driver,
        plan: &Plan,
        outcome: Option<&StepOutcome>,
        total: u64,
    ) -> Result<(), Error> {
        let digest = driver.observe(plan, outcome)?;
        let s = driver.sample();
        let tick = s.world.tick_index();
        let ranks = match &*driver {
            Driver::Simple(s) => {
                if self.simple_ranks.is_empty() {
                    self.simple_ranks = s
                        .handles
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(r, h)| (h, r))
                        .collect();
                }
                &self.simple_ranks
            }
            Driver::Burst(b) => &b.handle_ranks,
        };
        let world = &driver.sample().world;
        self.events.record(world, ranks, outcome)?;
        self.trace.write_all(&tick.to_le_bytes())?;
        self.trace.write_all(digest.as_bytes())?;
        self.trace_count += 1;
        if tick.is_multiple_of(1_024) || tick == total {
            self.checkpoints.push(json!({"tick":tick,"command_cursor":world.command_cursor(),"state_digest":state_digest(world)?}));
            self.trace.flush()?;
            self.journal.output.flush()?;
            eprintln!(
                "restore {:?} scale={} tick={tick}/{total}",
                plan.identity.case, plan.identity.scale
            );
        }
        Ok(())
    }
    fn finish(&mut self, output: &Path) -> Result<(), Error> {
        self.trace.flush()?;
        self.journal.output.flush()?;
        self.events.write(&output.join("events.bin"))?;
        crate::write_json(&output.join("batches.json"), &json!(self.batches))
    }
}

/// 文件长度和 SHA-256，供外部通用工具独立重算（ADR 0033）。
///
/// # Errors
/// 文件无法读取时返回 I/O 错误。
pub fn file_identity(path: &Path) -> Result<Value, Error> {
    let bytes = fs::read(path)?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(json!({"bytes":bytes.len(),"sha256":digest}))
}

fn read(path: &Path) -> Result<Value, Error> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn validate_implementation(implementation: &str) -> Result<(), Error> {
    require(
        implementation.len() == 40
            && implementation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        || "实现身份必须为完整小写提交 SHA".into(),
    )
}

fn positions(plan: &Plan, probe: bool) -> [(u64, bool); 2] {
    let w = plan.warm_up_ticks;
    match plan.identity.case {
        Case::W1 | Case::W2 => [(w, false), (if probe { w + 128 } else { 2 * w }, false)],
        Case::W3 => [(w + 1, false), (w + 1, true)],
        Case::W4 => [(w, false), (w + 1, false)],
    }
}

fn total(plan: &Plan, probe: bool) -> u64 {
    let full = plan.warm_up_ticks + plan.observation_ticks;
    if probe {
        full.min(positions(plan, true)[0].0 + 256)
    } else {
        full
    }
}

fn save(
    driver: &Driver,
    records: &Records,
    output: &Path,
    index: usize,
    after_commands: bool,
) -> Result<Value, Error> {
    let (host, bytes) = state::capture(driver, records, after_commands)?;
    let snapshot_name = format!("checkpoint-{index}.lfrs");
    let host_name = format!("checkpoint-{index}.json");
    fs::write(output.join(&snapshot_name), bytes)?;
    crate::write_json(&output.join(&host_name), &serde_json::to_value(&host)?)?;
    Ok(
        json!({"index":index,"tick":host.tick,"after_commands":after_commands,"host":file_identity(&output.join(host_name))?,"snapshot":file_identity(&output.join(snapshot_name))?}),
    )
}

fn report_files(output: &Path) -> Result<Value, Error> {
    let mut value = json!({});
    for name in [
        "ticks.bin",
        "events.bin",
        "commands.bin",
        "batches.json",
        "final.json",
    ] {
        value[name] = file_identity(&output.join(name))?;
    }
    Ok(value)
}

fn with_output(
    output: &Path,
    operation: impl FnOnce() -> Result<Value, Error>,
) -> Result<Value, Error> {
    fs::create_dir(output)?;
    crate::write_json(&output.join("report.json"), &json!({"complete":false}))?;
    match operation() {
        Ok(report) => {
            crate::write_json(&output.join("report.json"), &report)?;
            Ok(report)
        }
        Err(error) => {
            crate::write_json(
                &output.join("report.json"),
                &json!({"complete":false,"error":error.to_string()}),
            )?;
            Err(error)
        }
    }
}

/// 生成单 worker 正常参考及两个可恢复检查点。probe 只截短末段并明确标记。
///
/// # Errors
/// 输入/身份、运行语义、文件写入或输出目录已存在时失败；保留不完整回执。
pub fn reference(
    input: &Path,
    implementation: &str,
    probe: bool,
    output: &Path,
) -> Result<Value, Error> {
    validate_implementation(implementation)?;
    with_output(output, || {
        let start = Instant::now();
        let bundle = verify_directory(input)?;
        let plan = Plan::read(&bundle)?;
        let execution = ExecutionConfig::new(NonZeroU32::MIN);
        let mut driver = if matches!(plan.identity.case, Case::W3 | Case::W4) {
            Driver::Burst(Box::new(Burst::new(&bundle, &plan, execution)?))
        } else {
            Driver::Simple(Box::new(Sample::new(&bundle, execution)?))
        };
        let preparation = start.elapsed().as_secs_f64();
        let end = total(&plan, probe);
        let scheduled = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<Scheduled>, _>>()?;
        let mut records = Records::new(output, end, plan.identity.scale, 0)?;
        let mut archives = Vec::new();
        let mut outcome = None;
        let run = Instant::now();
        for tick in 0..=end {
            records.observe(&mut driver, &plan, outcome.as_ref(), end)?;
            for (index, (at, after)) in positions(&plan, probe).into_iter().enumerate() {
                if at == tick && !after {
                    archives.push(save(&driver, &records, output, index, false)?);
                }
            }
            if tick == end {
                break;
            }
            driver.commands(&plan, &scheduled, &mut records)?;
            for (index, (at, after)) in positions(&plan, probe).into_iter().enumerate() {
                if at == tick && after {
                    archives.push(save(&driver, &records, output, index, true)?);
                }
            }
            outcome = Some(
                driver
                    .sample_mut()
                    .world
                    .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                    .map_err(|e| failure("reference step", e))?,
            );
        }
        if !probe {
            driver.finish(&plan)?;
        }
        require(archives.len() == 2, || "检查点不完整".into())?;
        records.finish(output)?;
        crate::write_json(
            &output.join("final.json"),
            &json!({"state":state::projection(driver.sample())?,"host":driver.final_host(),"command_count":records.journal.count,"blocked":records.journal.blocked}),
        )?;
        Ok(
            json!({"format":"lf-synth-restore-reference-1","complete":true,"probe":probe,"implementation_commit":implementation,"input_manifest":bundle.manifest(),"workers":1,"warm_up_ticks":plan.warm_up_ticks,"observation_ticks":plan.observation_ticks,"end_tick":end,"trace_records":records.trace_count,"archives":archives,"checkpoints":records.checkpoints,"files":report_files(output)?,"timing":{"preparation_seconds":preparation,"run_seconds":run.elapsed().as_secs_f64(),"total_seconds":start.elapsed().as_secs_f64()}}),
        )
    })
}

fn checked_reference(
    bundle: &Bundle,
    plan: &Plan,
    implementation: &str,
    reference: &Path,
    index: usize,
) -> Result<(Value, state::Host), Error> {
    require(index < 2, || "检查点编号只能是 0 或 1".into())?;
    let report = read(&reference.join("report.json"))?;
    let probe = report["probe"]
        .as_bool()
        .ok_or_else(|| failure("probe flag", "missing"))?;
    require(
        report["format"] == "lf-synth-restore-reference-1"
            && report["complete"] == true
            && report["implementation_commit"] == implementation
            && report["input_manifest"] == serde_json::to_value(bundle.manifest())?
            && report["workers"] == 1
            && report["warm_up_ticks"] == plan.warm_up_ticks
            && report["observation_ticks"] == plan.observation_ticks
            && report["end_tick"] == total(plan, probe)
            && report["trace_records"] == total(plan, probe) + 1,
        || "参考来源、配置或完整窗口不符".into(),
    )?;
    require(report["files"] == report_files(reference)?, || {
        "参考文件身份不符".into()
    })?;
    let archives = report["archives"]
        .as_array()
        .ok_or_else(|| failure("archives", "missing"))?;
    require(archives.len() == 2, || "参考检查点数量不符".into())?;
    let archive = &archives[index];
    let host_file = reference.join(format!("checkpoint-{index}.json"));
    let snapshot_file = reference.join(format!("checkpoint-{index}.lfrs"));
    require(
        archive["index"] == index
            && archive["host"] == file_identity(&host_file)?
            && archive["snapshot"] == file_identity(&snapshot_file)?,
        || "检查点文件身份或编号不符".into(),
    )?;
    let host: state::Host = serde_json::from_slice(&fs::read(host_file)?)?;
    let position = positions(plan, probe)[index];
    require(
        (host.tick, host.after_commands) == position
            && archive["tick"] == position.0
            && archive["after_commands"] == position.1
            && host.trace_offset == TRACE_HEADER.len() as u64 + (host.tick + 1) * 40,
        || "检查点宿主阶段或轨迹边界不符".into(),
    )?;
    Ok((report, host))
}

fn suffix_equal(
    reference: &Path,
    candidate: &Path,
    header: &[u8],
    offset: usize,
) -> Result<(), Error> {
    let a = fs::read(reference)?;
    let b = fs::read(candidate)?;
    require(
        a.starts_with(header)
            && b.starts_with(header)
            && offset >= header.len()
            && offset <= a.len()
            && b[header.len()..] == a[offset..],
        || format!("完整后缀不同或文件截断：{}", candidate.display()),
    )
}

/// 从实际文件恢复后运行至参考声明终点，并比较全部后缀和末态。
///
/// # Errors
/// 来源、认证、恢复、阶段/映射/worker、轨迹/命令/末态或 I/O 不符时失败；不补跑。
pub fn resume(
    input: &Path,
    implementation: &str,
    execution: ExecutionConfig,
    reference: &Path,
    index: usize,
    output: &Path,
) -> Result<Value, Error> {
    validate_implementation(implementation)?;
    with_output(output, || {
        let start = Instant::now();
        let bundle = verify_directory(input)?;
        let plan = Plan::read(&bundle)?;
        let (reference_report, host) =
            checked_reference(&bundle, &plan, implementation, reference, index)?;
        let end = reference_report["end_tick"]
            .as_u64()
            .ok_or_else(|| failure("end tick", "missing"))?;
        let mut times = json!({"input_and_archive_seconds":start.elapsed().as_secs_f64()});
        let mut driver = state::restore(
            &bundle,
            &plan,
            &host,
            &fs::read(reference.join(format!("checkpoint-{index}.lfrs")))?,
            execution,
            &mut times,
        )?;
        let scheduled = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<Scheduled>, _>>()?;
        let mut records = Records::new(output, end, plan.identity.scale, host.tick + 1)?;
        records.journal.count = host.command_count;
        records.journal.blocked = host.blocked;
        let run = Instant::now();
        for boundary in host.tick..end {
            if boundary != host.tick || !host.after_commands {
                driver.commands(&plan, &scheduled, &mut records)?;
            }
            let outcome = driver
                .sample_mut()
                .world
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure("restored step", e))?;
            require(outcome.tick_index() == boundary + 1, || {
                "恢复继续运行跳拍".into()
            })?;
            records.observe(&mut driver, &plan, Some(&outcome), end)?;
        }
        times["continuation_seconds"] = json!(run.elapsed().as_secs_f64());
        if reference_report["probe"] == false {
            driver.finish(&plan)?;
        }
        require(
            driver.sample().world.tick_index() == end && records.trace_count == end - host.tick,
            || "恢复未达到完整终点".into(),
        )?;
        records.finish(output)?;
        let final_value = json!({"state":state::projection(driver.sample())?,"host":driver.final_host(),"command_count":records.journal.count,"blocked":records.journal.blocked});
        crate::write_json(&output.join("final.json"), &final_value)?;
        require(final_value == read(&reference.join("final.json"))?, || {
            "恢复末态或宿主累计记录不符".into()
        })?;
        suffix_equal(
            &reference.join("ticks.bin"),
            &output.join("ticks.bin"),
            TRACE_HEADER,
            host.trace_offset as usize,
        )?;
        suffix_equal(
            &reference.join("events.bin"),
            &output.join("events.bin"),
            EVENT_HEADER,
            host.event_offset,
        )?;
        suffix_equal(
            &reference.join("commands.bin"),
            &output.join("commands.bin"),
            COMMAND_HEADER,
            COMMAND_HEADER.len() + host.command_count as usize * 68,
        )?;
        let batches = read(&reference.join("batches.json"))?;
        let batches = batches
            .as_array()
            .ok_or_else(|| failure("batches", "not array"))?;
        require(
            host.batch_count <= batches.len() && records.batches == batches[host.batch_count..],
            || "命令批次后缀不同".into(),
        )?;
        let expected_points = reference_report["checkpoints"]
            .as_array()
            .ok_or_else(|| failure("checkpoints", "missing"))?
            .iter()
            .filter(|p| p["tick"].as_u64().is_some_and(|t| t > host.tick))
            .cloned()
            .collect::<Vec<_>>();
        require(records.checkpoints == expected_points, || {
            "恢复状态检查点后缀不符".into()
        })?;
        times["total_seconds"] = json!(start.elapsed().as_secs_f64());
        Ok(
            json!({"format":"lf-synth-restore-run-1","complete":true,"probe":reference_report["probe"],"implementation_commit":implementation,"input_manifest":bundle.manifest(),"workers":driver.sample().world.execution_config().worker_count().get(),"checkpoint":index,"start_tick":host.tick,"after_commands":host.after_commands,"end_tick":end,"trace_records":records.trace_count,"command_count":records.journal.count,"blocked":records.journal.blocked,"attestation":"Pass","new_observation_stream":true,"initial_state_and_mapping_equal":true,"full_suffix_equal":true,"final_state_equal":true,"checkpoints":records.checkpoints,"files":report_files(output)?,"timing":times}),
        )
    })
}

#[cfg(test)]
mod tests;
