use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Artifacts, Harness, ResolvedPlan, Result, TickRecord, invalid, observe, runner::TileEvidence,
    sha256,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunResult {
    pub version: String,
    pub status: String,
    pub purpose: String,
    pub case: String,
    pub scale: String,
    pub network_revision: String,
    pub policy_id: String,
    pub world_id: u64,
    pub fixed_step_ms: u64,
    pub window: crate::Window,
    pub required_per_tile: BTreeMap<String, u64>,
    pub plan_digest: String,
    pub expected_ticks: u64,
    pub completed_ticks: u64,
    pub committed_world_tick: u64,
    pub error: Option<String>,
    pub checkpoints: BTreeMap<u64, String>,
    pub initial_counts: (usize, usize, usize),
    pub final_counts: (usize, usize, usize),
    pub tile_evidence: Vec<TileEvidence>,
    pub retry_reasons: BTreeMap<String, u64>,
    pub atomic_rejections: BTreeMap<String, u32>,
    pub replacements: u64,
    pub pending_departures: usize,
    pub exhausted_departures: usize,
    pub files: BTreeMap<String, crate::artifacts::FileDigest>,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn line(file: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Runs one fresh world. A failed run still leaves result.json with the last committed tick.
/// Step timing is diagnostic only; this slice does not implement the performance protocol.
pub fn run_to_directory(
    artifacts: &Artifacts,
    plan: &ResolvedPlan,
    output: &Path,
) -> Result<RunResult> {
    plan.validate(artifacts)?;
    fs::create_dir(output)?;
    let execution_id = new_execution_id()?;
    let plan_digest = plan.write(&output.join("resolved-plan.toml"))?;
    let started = Instant::now();
    let mut harness = Harness::install(artifacts, plan)?;
    let initial_counts = observe::counts(&harness)?;
    let mut result = RunResult {
        version: "urban-result-v2".into(),
        status: "failed".into(),
        purpose: plan.window.purpose.clone(),
        case: plan.case.clone(),
        scale: plan.scale.clone(),
        network_revision: artifacts.catalog.network_revision.clone(),
        policy_id: artifacts.catalog.policy_id.clone(),
        world_id: harness.world.world_id(),
        fixed_step_ms: plan.dt,
        window: plan.window.clone(),
        required_per_tile: plan.required_per_tile.clone(),
        plan_digest,
        expected_ticks: plan.window.end(),
        completed_ticks: 0,
        committed_world_tick: 0,
        error: None,
        checkpoints: BTreeMap::new(),
        initial_counts,
        final_counts: initial_counts,
        tile_evidence: Vec::new(),
        retry_reasons: BTreeMap::new(),
        atomic_rejections: BTreeMap::new(),
        replacements: 0,
        pending_departures: 0,
        exhausted_departures: 0,
        files: BTreeMap::new(),
    };
    result.checkpoints.insert(0, harness.checkpoint()?);
    let mut ticks = BufWriter::new(File::create(output.join("ticks.jsonl"))?);
    let mut commands = BufWriter::new(File::create(output.join("commands.jsonl"))?);
    let mut events = BufWriter::new(File::create(output.join("events.jsonl"))?);
    let mut times = Vec::new();
    let run: Result<()> = (|| {
        for _ in 0..plan.window.end() {
            let record = harness.advance()?;
            line(&mut ticks, &record)?;
            for command in &harness.commands {
                line(&mut commands, command)?;
            }
            for event in &harness.events {
                line(&mut events, event)?;
            }
            times.push(harness.last_step_ns);
            result.completed_ticks = record.tick;
            result.final_counts = (record.active, record.parked, record.completed);
            result.pending_departures = record.pending_departures;
            result.exhausted_departures = record.exhausted_departures;
            if record.tick == plan.window.warm_up_ticks
                || record.tick == plan.window.end()
                || (record.tick > plan.window.warm_up_ticks
                    && (record.tick - plan.window.warm_up_ticks).is_multiple_of(plan.cycle_ticks))
            {
                result
                    .checkpoints
                    .insert(record.tick, harness.checkpoint()?);
            }
            if record.tick.is_multiple_of(1_024) {
                eprintln!(
                    "tick {}/{}: active={} parked={} completed={} elapsed={:.1}s",
                    record.tick,
                    plan.window.end(),
                    record.active,
                    record.parked,
                    record.completed,
                    started.elapsed().as_secs_f64()
                );
            }
        }
        if plan.window.purpose == "correctness" {
            for (tile, e) in harness.evidence.iter().enumerate() {
                if e.crossed_tile_completed < plan.required_per_tile["crossed_tile_completed"]
                    || e.red_wait_then_crossed < plan.required_per_tile["red_wait_then_crossed"]
                    || e.leaves + e.explicit_parks + e.virtual_parks
                        < plan.required_per_tile["park_or_leave"]
                {
                    return Err(invalid(format!(
                        "tile {tile}: missing required MIXED-PEAK observation"
                    )));
                }
                if e.planned_east * 3 != e.planned_west * 7 {
                    return Err(invalid(format!(
                        "tile {tile}: departure input is not 70:30"
                    )));
                }
            }
            result.status = "case-pass-replay-required".into();
        } else {
            result.status = "probe-complete".into();
        }
        Ok(())
    })();
    if let Err(error) = run {
        result.error = Some(error.to_string());
    }
    // Keep committed observations from a failed advance visible, without marking that tick passed.
    if result.error.is_some() {
        write_json(
            &output.join("failure.json"),
            &json!({"committed_world_tick":harness.world.tick_index(),
            "commands":harness.commands, "events":harness.events, "error":result.error}),
        )?;
    }
    ticks.flush()?;
    commands.flush()?;
    events.flush()?;
    result.tile_evidence = harness.evidence.clone();
    result.retry_reasons = harness.error_counts.clone();
    result.atomic_rejections = harness.atomic_rejections.clone();
    result.replacements = harness.replacements;
    result.committed_world_tick = harness.world.tick_index();
    for name in [
        "ticks.jsonl",
        "commands.jsonl",
        "events.jsonl",
        "resolved-plan.toml",
    ] {
        result
            .files
            .insert(name.into(), digest_file(&output.join(name))?);
    }
    write_json(&output.join("result.json"), &result)?;
    times.sort_unstable();
    let percentile = |n: usize| {
        times
            .get((times.len() * n).div_ceil(100).saturating_sub(1))
            .copied()
    };
    write_json(
        &output.join("diagnostics.json"),
        &json!({"purpose":"diagnostic-only-not-performance-certification", "execution_id":execution_id, "elapsed_seconds":started.elapsed().as_secs_f64(),
        "verified_steps":times.len(), "step_ns_p50":percentile(50), "step_ns_p95":percentile(95), "step_ns_p99":percentile(99),
        "os":std::env::consts::OS, "architecture":std::env::consts::ARCH, "workers":1,
        "cpu":std::env::var("PROCESSOR_IDENTIFIER").ok(), "logical_cpus":std::thread::available_parallelism().map(|n| n.get()).ok(),
        "binary":std::env::current_exe().ok().and_then(|path| digest_file(&path).ok()),
        "invocation":std::env::args().collect::<Vec<_>>(), "memory_measurement":null,
        "git_commit_at_run":command_output("git", &["rev-parse", "HEAD"]),
        "git_status_at_run":command_output("git", &["status", "--porcelain"])}),
    )?;
    Ok(result)
}

fn digest_file(path: &Path) -> Result<crate::artifacts::FileDigest> {
    use sha2::{Digest, Sha256};
    let mut reader = BufReader::new(File::open(path)?);
    let mut hash = Sha256::new();
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            break;
        }
        hash.update(bytes);
        let len = bytes.len();
        reader.consume(len);
    }
    Ok(crate::artifacts::FileDigest {
        bytes: fs::metadata(path)?.len(),
        sha256: crate::hex(&hash.finalize()),
    })
}

/// Verifies retained files and semantic results, returning the verified completion kind.
pub fn compare_runs(left: &Path, right: &Path) -> Result<String> {
    if fs::canonicalize(left)? == fs::canonicalize(right)? {
        return Err(invalid("replay requires two distinct run directories"));
    }
    if read_execution_id(left)? == read_execution_id(right)? {
        return Err(invalid("replay requires distinct execution identities"));
    }
    let read = |dir: &Path| -> Result<RunResult> {
        let result: RunResult = serde_json::from_slice(&fs::read(dir.join("result.json"))?)?;
        if result.version != "urban-result-v2"
            || result.error.is_some()
            || result.completed_ticks != result.expected_ticks
            || !matches!(
                (result.purpose.as_str(), result.status.as_str()),
                ("probe", "probe-complete") | ("correctness", "case-pass-replay-required")
            )
        {
            return Err(invalid("cannot accept an incomplete or failed run"));
        }
        for name in [
            "resolved-plan.toml",
            "ticks.jsonl",
            "commands.jsonl",
            "events.jsonl",
        ] {
            if result.files.get(name) != Some(&digest_file(&dir.join(name))?) {
                return Err(invalid(format!("run file changed: {name}")));
            }
        }
        if sha256(&fs::read(dir.join("resolved-plan.toml"))?) != result.plan_digest {
            return Err(invalid("plan digest differs"));
        }
        let mut count = 0;
        for line in BufReader::new(File::open(dir.join("ticks.jsonl"))?).lines() {
            let row: TickRecord = serde_json::from_str(&line?)?;
            count += 1;
            if row.tick != count || row.active + row.parked + row.completed != row.live {
                return Err(invalid("tick sequence or lifecycle differs"));
            }
        }
        if count != result.completed_ticks {
            return Err(invalid("tick log is incomplete"));
        }
        Ok(result)
    };
    let a = read(left)?;
    let b = read(right)?;
    if a.plan_digest != b.plan_digest {
        return Err(invalid("different resolved plans"));
    }
    if a != b {
        for (index, (left_row, right_row)) in BufReader::new(File::open(left.join("ticks.jsonl"))?)
            .lines()
            .zip(BufReader::new(File::open(right.join("ticks.jsonl"))?).lines())
            .enumerate()
        {
            let left_row = left_row?;
            let right_row = right_row?;
            if left_row != right_row {
                return Err(invalid(format!(
                    "first differing tick {}: left={left_row}; right={right_row}; inspect commands/events at the preceding boundary",
                    index + 1
                )));
            }
        }
        return Err(invalid(
            "independent runs differ; compare ticks and semantic logs",
        ));
    }
    Ok(if a.purpose == "correctness" {
        "MIXED-PEAK replay passed; #544 full matrix remains open"
    } else {
        "probe replay matched; not formal case acceptance"
    }
    .into())
}

// Local copy/mix-up detection only; this metadata does not attest execution or enter semantic hashes.
fn new_execution_id() -> Result<String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| invalid(format!("execution start time unavailable: {error}")))?;
    Ok(format!(
        "{}-{}-{}",
        std::process::id(),
        started.as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn read_execution_id(directory: &Path) -> Result<String> {
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("diagnostics.json"))?)?;
    diagnostics["execution_id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("missing execution identity; rerun to produce current evidence"))
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
