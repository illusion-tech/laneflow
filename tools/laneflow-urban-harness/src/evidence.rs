//! #545 evidence uses the existing scheduler/oracle; it has no second traffic model.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::{Duration, Instant},
};

use laneflow_spatial::SpatialSession;
use serde_json::{Value, json};

use crate::{
    Artifacts, Harness, Presentation, ResolvedPlan, Result, checked, invalid, observe,
    report::{
        command_output, digest_file, new_execution_id, peak_resident_bytes, peak_resident_method,
        sample_summary, validate_case, write_json,
    },
};

const WRITE_RESERVE_MS: u64 = 10_000;

fn stop_reason(
    completed: u64,
    target: u64,
    elapsed: Duration,
    soft: Option<Duration>,
) -> Option<&'static str> {
    if soft.is_some_and(|limit| elapsed >= limit) {
        Some("wall-limit")
    } else if completed >= target {
        Some("tick-limit")
    } else {
        None
    }
}

fn target_ticks(planned: u64, wall_ms: Option<u64>, prefix: Option<u64>) -> Result<u64> {
    match prefix {
        Some(ticks) if wall_ms.is_some() && (1..=planned).contains(&ticks) => Ok(ticks),
        Some(_) => Err(invalid(
            "a tick prefix requires bounded evidence and must be within the unchanged plan",
        )),
        None => Ok(planned),
    }
}

pub(crate) fn source() -> Result<Value> {
    let read = |program, args: &[&str]| {
        command_output(program, args)
            .ok_or_else(|| invalid(format!("unavailable source metadata: {program}")))
    };
    let commit = read("git", &["rev-parse", "HEAD"])?;
    let status = read("git", &["status", "--porcelain"])?;
    if !status.is_empty() || commit.len() != 40 {
        return Err(invalid("evidence requires a frozen clean source checkout"));
    }
    Ok(json!({"commit":commit,"git_status":status,
        "rustc":read("rustc", &["+1.98.0","-Vv"] )?,
        "cargo":read("cargo", &["+1.98.0","-V"] )?,
        "binary":digest_file(&std::env::current_exe()?)?,
        "build_parameters":"cargo +1.98.0 build -p laneflow-urban-harness --features adapter --release --locked",
        "hardware_role":std::env::var("LANEFLOW_HARDWARE_ROLE").map_err(|_| invalid("missing hardware role"))?,
        "power_role":std::env::var("LANEFLOW_POWER_ROLE").map_err(|_| invalid("missing power role"))?,
        "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"workers":1}))
}

/// Correctness consumes the complete existing plan. A wall budget is a separate,
/// explicitly truncated Mixed observation, never a formal three-round result.
pub fn run_evidence(
    artifact_directory: &Path,
    plan_path: &Path,
    output: &Path,
    adapter: bool,
    wall_ms: Option<u64>,
    prefix_ticks: Option<u64>,
) -> Result<Value> {
    let started = Instant::now();
    let soft = wall_ms
        .map(|limit| {
            if !(WRITE_RESERVE_MS + 1..=600_000).contains(&limit) {
                return Err(invalid(
                    "wall budget must be greater than 10000 and at most 600000 ms",
                ));
            }
            Ok(Duration::from_millis(limit - WRITE_RESERVE_MS))
        })
        .transpose()?;
    let provenance = source()?;
    let artifacts = if adapter {
        Artifacts::load_spatial(artifact_directory)?
    } else {
        Artifacts::load(artifact_directory)?
    };
    let plan = ResolvedPlan::read(plan_path)?;
    plan.validate(&artifacts)?;
    if wall_ms.is_some() {
        if plan.case != "MIXED-PEAK"
            || plan.window.purpose != "probe"
            || plan.window.warm_up_ticks != 0
            || plan.window.end() > 4_096
            || !matches!(plan.scale.as_str(), "10k" | "100k")
        {
            return Err(invalid(
                "bounded evidence requires a 10k/100k Mixed probe of at most 4096 ticks without warmup",
            ));
        }
    } else if plan.window.purpose != "correctness" {
        return Err(invalid(
            "correctness evidence requires the complete accepted correctness window",
        ));
    }
    let target = target_ticks(plan.window.end(), wall_ms, prefix_ticks)?;
    fs::create_dir(output)?;
    let plan_digest = plan.write(&output.join("resolved-plan.toml"))?;
    let mut harness = Harness::install(&artifacts, &plan)?;
    if adapter {
        let spatial = checked(
            "Spatial bind",
            SpatialSession::bind(artifacts.revision().clone()),
        )?
        .ok_or_else(|| invalid("canonical geometry is absent"))?;
        harness = harness.into_adapter(spatial)?;
    }
    let mut presentation = adapter.then(Presentation::default);
    let initial_counts = observe::counts(&harness)?;
    let initial_checkpoint = harness.checkpoint()?;
    let initialized_ms = started.elapsed().as_millis();
    let mut frames = BufWriter::new(File::create(output.join("frames.jsonl"))?);
    let mut steps = Vec::new();
    let mut command_times = Vec::new();
    let mut observations = Vec::new();
    let mut pose_times = Vec::new();
    let mut selection_times = Vec::new();
    let mut apply_times = Vec::new();
    let mut frame_times = Vec::new();
    let mut last_presentation = None;
    let mut checkpoints = BTreeMap::from([(0, initial_checkpoint)]);
    let mut completed = 0;
    let mut reason = "failure";
    let run: Result<()> = (|| {
        loop {
            if let Some(stop) = stop_reason(completed, target, started.elapsed(), soft) {
                reason = stop;
                break;
            }
            let frame_started = Instant::now();
            let mut record = harness.advance()?;
            let sample = presentation
                .as_mut()
                .map(|p| p.sample(&mut harness))
                .transpose()?;
            if let Some(sample) = &sample {
                record.presented = sample.n_presented;
                pose_times.push(sample.pose_ns);
                selection_times.push(sample.selection_mapping_ns);
                apply_times.push(sample.apply_ns);
                last_presentation = Some(sample.clone());
            }
            let frame_ns = frame_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            serde_json::to_writer(
                &mut frames,
                &json!({"runtime":record,"presentation":sample,
                "commands":harness.commands,"events":harness.events,
                "command_ns":harness.last_command_ns,"step_ns":harness.last_step_ns,
                "observation_ns":harness.last_observation_ns,"frame_ns":frame_ns}),
            )?;
            frames.write_all(b"\n")?;
            steps.push(harness.last_step_ns);
            command_times.push(harness.last_command_ns);
            observations.push(harness.last_observation_ns);
            frame_times.push(frame_ns);
            completed = record.tick;
            if wall_ms.is_none()
                && (completed == plan.window.warm_up_ticks
                    || completed > plan.window.warm_up_ticks
                        && (completed - plan.window.warm_up_ticks).is_multiple_of(plan.cycle_ticks))
            {
                checkpoints.insert(completed, harness.checkpoint()?);
            }
            if completed.is_multiple_of(256) {
                eprintln!(
                    "{} {} tick {}/{} active={} presented={} elapsed={:.1}s",
                    plan.scale,
                    if adapter { "Adapter" } else { "headless" },
                    completed,
                    target,
                    record.active,
                    record.presented,
                    started.elapsed().as_secs_f64()
                );
            }
        }
        if completed == 0 {
            return Err(invalid("empty observation window"));
        }
        if wall_ms.is_none() {
            validate_case(&harness)?;
        }
        checkpoints.insert(completed, harness.checkpoint()?);
        if source()? != provenance {
            return Err(invalid("source changed during evidence run"));
        }
        for (name, expected) in &artifacts.files {
            if digest_file(&artifact_directory.join(name))? != *expected {
                return Err(invalid(format!(
                    "input changed during evidence run: {name}"
                )));
            }
        }
        if digest_file(&artifact_directory.join("manifest.toml"))?.sha256
            != artifacts.manifest_digest
        {
            return Err(invalid("input manifest changed during evidence run"));
        }
        Ok(())
    })();
    frames.flush()?;
    let mut error = run.err().map(|e| e.to_string());
    let memory = peak_resident_bytes();
    let summary = |values: &mut [u64]| -> Result<Value> {
        if values.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::to_value(sample_summary(values)?)?)
        }
    };
    let mut files = BTreeMap::new();
    for name in ["frames.jsonl", "resolved-plan.toml"] {
        files.insert(name.to_owned(), digest_file(&output.join(name))?);
    }
    if let Some(presentation) = &presentation {
        let pose = artifacts
            .revision()
            .spatial()
            .and_then(|s| s.lane_pose())
            .ok_or_else(|| invalid("preview canonical geometry absent"))?;
        let mut roads = Vec::new();
        for ordinal in artifacts.edges.values() {
            let geometry = pose
                .lane_geometry(*ordinal)
                .ok_or_else(|| invalid("preview lane absent"))?;
            if geometry.canonical_frame().index() != 0 {
                return Err(invalid("preview requires the single canonical frame"));
            }
            roads.push(
                geometry
                    .points()
                    .iter()
                    .map(|p| [p.x, p.y, p.z])
                    .collect::<Vec<_>>(),
            );
        }
        write_json(
            &output.join("preview.json"),
            &json!({"kind":"applied-transform-positions",
            "scope":last_presentation,"positions":presentation.preview(),"canonical_roads_m":roads,
            "tiles":artifacts.tiles,"cells":artifacts.tiles*10,"scale":plan.scale,
            "network_revision":artifacts.catalog.network_revision,"drawing_scope":"top-down projection of canonical lane geometry and applied Transform positions; no GPU draw benchmark"}),
        )?;
        files.insert(
            "preview.json".into(),
            digest_file(&output.join("preview.json"))?,
        );
    }
    if wall_ms.is_some_and(|limit| started.elapsed() >= Duration::from_millis(limit)) {
        error = Some("process exceeded its assigned wall budget".into());
    }
    let measurements = json!({"step":summary(&mut steps)?,"commands":summary(&mut command_times)?,
        "observation":summary(&mut observations)?,"pose":summary(&mut pose_times)?,
        "selection_mapping":summary(&mut selection_times)?,"apply":summary(&mut apply_times)?,"frame":summary(&mut frame_times)?});
    let mut result = json!({
        "version":"urban-cross-layer-evidence-v1",
        "status":if error.is_some() {"failed"} else if wall_ms.is_some() {"bounded-observation-complete"} else {"correctness-case-pass"},
        "execution_id":new_execution_id()?,"pid":std::process::id(),"source":provenance,
        "invocation":std::env::args().collect::<Vec<_>>(),"case":plan.case,"scale":plan.scale,
        "mode":if adapter {"adapter"} else {"headless"},"window":plan.window,
        "plan_digest":plan_digest,"artifact_files":artifacts.files,"manifest_sha256":artifacts.manifest_digest,
        "network_revision":artifacts.catalog.network_revision,"policy_id":artifacts.catalog.policy_id,
        "target_ticks":target,"prefix_ticks":prefix_ticks,"completed_ticks":completed,"committed_world_tick":harness.world().tick_index(),
        "wall_limit_ms":wall_ms,"soft_limit_ms":soft.map(|v|v.as_millis()),"stop_reason":reason,
        "initialized_ms":initialized_ms,"elapsed_ms_before_result_write":started.elapsed().as_millis(),
        "initial_counts":initial_counts,"final_counts":observe::counts(&harness)?,"last_presentation":last_presentation,
    });
    let details = json!({
        "checkpoints":checkpoints,"tile_evidence":harness.evidence,"retry_reasons":harness.error_counts,
        "atomic_rejections":harness.atomic_rejections,"births":harness.births,"removals":harness.removals,"replacements":harness.replacements,
        "timing_basis":if adapter {"step=Bevy fixed Step stage including scheduler boundaries; pose=full source materialization and extraction; selection_mapping=stable selection and proxy lifecycle; apply=Transform writes; validation and log IO separate"} else {"step=TrafficWorld::step public call; command=public lifecycle calls; observation=existing oracle"},
        "frame_basis":"commands, Runtime/Bevy step, oracle, full presentation and its validation; excludes frame log serialization and checkpoints; percentiles are not additive",
        "measurements_ns":measurements,
        "memory":{"status":if memory.is_some(){"measured"}else{"unmeasured"},"method":peak_resident_method(),"peak_resident_bytes":memory},
        "files":files,"error":error,"product_certification":false,
    });
    result
        .as_object_mut()
        .expect("object")
        .extend(details.as_object().expect("object").clone());
    write_json(&output.join("evidence.json"), &result)?;
    if let Some(error) = result["error"].as_str() {
        return Err(invalid(error));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_cannot_shorten_correctness_or_run_past_the_frozen_plan() {
        assert!(target_ticks(4_096, None, Some(512)).is_err());
        assert!(target_ticks(4_096, Some(120_000), Some(0)).is_err());
        assert!(target_ticks(4_096, Some(120_000), Some(4_097)).is_err());
        assert_eq!(target_ticks(4_096, Some(120_000), Some(512)).unwrap(), 512);
        assert_eq!(target_ticks(4_096, None, None).unwrap(), 4_096);
    }

    #[test]
    fn deadline_includes_initialization_and_wins_at_the_final_tick() {
        let soft = Some(Duration::from_millis(20));
        assert_eq!(
            stop_reason(0, 4_096, Duration::from_millis(20), soft),
            Some("wall-limit")
        );
        assert_eq!(
            stop_reason(4_096, 4_096, Duration::from_millis(20), soft),
            Some("wall-limit")
        );
        assert_eq!(
            stop_reason(4_096, 4_096, Duration::from_millis(19), soft),
            Some("tick-limit")
        );
        assert_eq!(stop_reason(1, 4_096, Duration::from_millis(19), soft), None);
    }
}
