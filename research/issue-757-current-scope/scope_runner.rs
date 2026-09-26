//! #757 原计划有限前缀；四臂使用相同观测，不参与正式性能比较协议。
use crate::{Artifacts, Harness, ResolvedPlan, Result, invalid};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

/// 执行有限前缀，并保留输入接纳、需求、交通质量和逐拍墙钟。
/// # Errors
/// 输入、安装、推进、验证或写出失败时保留已写证据并返回错误。
pub fn run_scope_prefix(
    artifacts: &Path,
    plan_path: &Path,
    output: &Path,
    workers: u32,
    limit: u64,
) -> Result<()> {
    let mode = std::env::var("LF757_SCOPE").unwrap_or_else(|_| "all".into());
    let run_id = std::env::var("LF757_RUN_ID").map_err(|_| invalid("missing run identity"))?;
    if run_id.is_empty() {
        return Err(invalid("empty run identity"));
    }
    if !["all", "p3", "waiting", "both"].contains(&mode.as_str())
        || !(1..=crate::MAX_WORKERS).contains(&workers)
    {
        return Err(invalid("invalid mode or workers"));
    }
    let counting = laneflow_runtime::research_scope_counting_enabled();
    if counting != std::env::var("LF757_DIAGNOSTIC").is_ok() {
        return Err(invalid(
            "diagnostic flag differs from compiled counting mode",
        ));
    }
    if counting && workers != 1 {
        return Err(invalid("thread-local diagnostics require one worker"));
    }
    let artifacts = Artifacts::load(artifacts)?;
    let plan = ResolvedPlan::read(plan_path)?;
    plan.validate(&artifacts)?;
    let run_identity = serde_json::json!({
        "run_id":run_id,"mode":mode,"scale":plan.scale,"workers":workers,"counting":counting,"limit":limit,
        "plan_sha256":crate::sha256(&fs::read(plan_path)?),"input_manifest_sha256":artifacts.manifest_digest,
        "quality_schema":"active-to-active-v1"
    });
    if limit == 0 || limit >= plan.window.end() {
        return Err(invalid("prefix must be inside original plan"));
    }
    fs::create_dir(output)?;
    let execution =
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap());
    let mut h = Harness::install(&artifacts, &plan, execution)?;
    let initial_checkpoint = h.checkpoint()?;
    let mut initial = BufWriter::new(File::create(output.join("initial.jsonl"))?);
    for individual in &h.individuals {
        let vehicle = individual
            .handle
            .ok_or_else(|| invalid("missing initial handle"))?;
        serde_json::to_writer(
            &mut initial,
            &serde_json::json!({"id":individual.id,
            "state":format!("{:?}",h.world.vehicle(vehicle)),
            "parking":format!("{:?}",h.world.parking_binding(vehicle))}),
        )?;
        writeln!(initial)?;
    }
    initial.flush()?;
    let initial_counts = crate::observe::counts(&h)?;
    let mut quality = crate::research_quality::Quality::new(&h, output)?;
    let mut timing = BufWriter::new(File::create(output.join("timing.csv"))?);
    let mut ticks = BufWriter::new(File::create(output.join("ticks.jsonl"))?);
    let mut commands = BufWriter::new(File::create(output.join("commands.jsonl"))?);
    let mut events = BufWriter::new(File::create(output.join("events.jsonl"))?);
    let mut counts = BufWriter::new(File::create(output.join("work.csv"))?);
    writeln!(
        timing,
        "tick,active,intent,completed,command_ns,step_ns,base_observation_ns,quality_ns,iteration_ns"
    )?;
    writeln!(
        counts,
        "tick,waiting_entries,waiting_skipped,waiting_full_preview,p3_precount_visits,p3_considered,p3_skipped,p3_evaluated,p3_state_copy_bytes,motion_calculations"
    )?;
    laneflow_runtime::research_scope_counts();
    let mut last_record = None;
    let mut failure = None;
    for _ in 0..limit {
        let start = Instant::now();
        let record = match h.advance() {
            Ok(record) => record,
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
        };
        let quality_start = Instant::now();
        quality.observe(&h)?;
        let quality_ns = quality_start.elapsed().as_nanos();
        serde_json::to_writer(&mut ticks, &record)?;
        writeln!(ticks)?;
        for row in &h.commands {
            serde_json::to_writer(&mut commands, row)?;
            writeln!(commands)?;
        }
        for row in &h.events {
            serde_json::to_writer(&mut events, row)?;
            writeln!(events)?;
        }
        let iteration_ns = start.elapsed().as_nanos();
        writeln!(
            timing,
            "{},{},{},{},{},{},{},{quality_ns},{iteration_ns}",
            record.tick,
            record.active,
            record.intent,
            record.completed,
            h.last_command_ns,
            h.last_step_ns,
            h.last_observation_ns
        )?;
        write!(counts, "{}", record.tick)?;
        for value in laneflow_runtime::research_scope_counts() {
            write!(counts, ",{value}")?;
        }
        writeln!(counts)?;
        if record.tick.is_multiple_of(512) {
            eprintln!("{} {} tick {}/{limit}", plan.scale, mode, record.tick);
        }
        last_record = Some(record);
    }
    timing.flush()?;
    ticks.flush()?;
    commands.flush()?;
    events.flush()?;
    counts.flush()?;
    quality.finish(&h, output, &run_identity)?;
    let checkpoint = h.checkpoint();
    let peak_resident_bytes = crate::report::peak_resident_bytes();
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "run_identity":run_identity,
            "quality_sha256":crate::sha256(&fs::read(output.join("extended-quality.json"))?),
            "trips_sha256":crate::sha256(&fs::read(output.join("individual-quality.csv"))?),
            "status":if failure.is_none() { "research-prefix-complete" } else { "research-prefix-failed" },
            "mode":mode,"workers":workers,"counting":counting,"limit":limit,"expected_plan_end":plan.window.end(),"peak_resident_bytes":peak_resident_bytes,
            "plan_sha256":crate::sha256(&fs::read(plan_path)?),"input_manifest_sha256":artifacts.manifest_digest,
            "initial_checkpoint":initial_checkpoint,"initial_active":initial_counts.0,"initial_parked":initial_counts.1,
            "initial_completed":initial_counts.2,"initial_accepted":h.plan.initial.len(),
            "completed_ticks":h.world.tick_index(),"final_record":last_record,"checkpoint":checkpoint.as_ref().ok(),
            "error":failure,"error_counts":h.error_counts,"replacements":h.replacements,"births":h.births,"removals":h.removals,
            "evidence":h.evidence,"boundary":"research only; iteration includes quality and traffic JSON writing, excludes timing/work CSV writes and final flush; not formal certification"
        }))?,
    )?;
    if let Some(error) = failure {
        return Err(invalid(error));
    }
    checkpoint?;
    Ok(())
}
