use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use super::{ActiveLoadSummary, RetainedMeasurements, RunResult, sample_summary};
use crate::{ResolvedPlan, Result, TickRecord, Window, invalid};

fn target(result: &RunResult) -> Result<Option<u64>> {
    if result.purpose != "performance" || result.case != "SUSTAINED-ACTIVE" {
        return Ok(None);
    }
    match result.scale.as_str() {
        "10k" => Ok(Some(10_000)),
        "100k" => Ok(Some(100_000)),
        _ => Err(invalid(
            "sustained performance requires 10k or 100k artifacts",
        )),
    }
}

pub(super) fn validate_result(result: &RunResult) -> Result<()> {
    let Some(target) = target(result)? else {
        return Ok(());
    };
    let load = result
        .active_load
        .as_ref()
        .ok_or_else(|| invalid("missing sustained performance load"))?;
    if result.initial_counts != (target as usize, 0, 0)
        || result.final_counts.1 != 0
        || result.final_counts.0.checked_add(result.final_counts.2) != Some(target as usize)
        || result.replacements != result.births
        || result.replacements != result.removals
        || result.exhausted_departures != 0
        || load.target != target
        || load.before_step.samples as u64 != result.window.observation_ticks
        || load.after_step.samples as u64 != result.window.observation_ticks
        || load.before_step.min != target
        || load.before_step.max != target
        || load.before_step_below_target_ticks != 0
    {
        return Err(invalid(
            "持续 Active 正式负载门槛未通过；保留实际负载与测量，不认证预算",
        ));
    }
    Ok(())
}

pub(super) fn verify(
    directory: &Path,
    result: &RunResult,
    measurement: &RetainedMeasurements,
) -> Result<()> {
    let Some(target) = target(result)? else {
        return Ok(());
    };
    validate_result(result)?;
    let plan = ResolvedPlan::read(&directory.join("resolved-plan.toml"))?;
    plan.validate_structure()?;
    if plan.version != "urban-demand-v3"
        || plan.case != result.case
        || plan.scale != result.scale
        || plan.dt != result.fixed_step_ms
        || u64::from(plan.individuals) != target
        || plan.initial.len() as u64 != target
        || u64::from(plan.initial_counts.active) != target
        || plan.recycling.is_none()
        || plan.window != result.window
        || plan.window != Window::performance_for_cycle(plan.cycle_ticks)?
        || result.expected_ticks != plan.window.end()
        || result.committed_world_tick != result.completed_ticks
    {
        return Err(invalid(
            "sustained performance plan or complete window differs",
        ));
    }
    let checkpoints: BTreeSet<_> = std::iter::successors(Some(plan.window.warm_up_ticks), |tick| {
        tick.checked_add(plan.cycle_ticks)
    })
    .take_while(|&tick| tick <= plan.window.end())
    .chain([0, plan.window.end()])
    .collect();
    if result.checkpoints.keys().copied().collect::<BTreeSet<_>>() != checkpoints {
        return Err(invalid("sustained performance checkpoints are incomplete"));
    }
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut last = None;
    let mut tick_count = 0;
    for line in BufReader::new(File::open(directory.join("ticks.jsonl"))?).lines() {
        let row: TickRecord = serde_json::from_str(&line?)?;
        tick_count += 1;
        if row.tick != tick_count
            || row.live as u64 != target
            || row.intent > row.live
            || row
                .active
                .checked_add(row.parked)
                .and_then(|n| n.checked_add(row.completed))
                != Some(row.live)
            || row.intent_basis != "exact_active_before_step"
        {
            return Err(invalid(
                "sustained performance tick load or lifecycle differs",
            ));
        }
        if plan.window.contains_completed_step(row.tick) {
            before.push(row.intent as u64);
            after.push(row.active as u64);
        }
        last = Some(row);
    }
    let last = last.ok_or_else(|| invalid("missing sustained performance ticks"))?;
    if tick_count != result.completed_ticks
        || (last.active, last.parked, last.completed) != result.final_counts
        || last.pending_departures != result.pending_departures
        || last.exhausted_departures != result.exhausted_departures
        || before.len() as u64 != plan.window.observation_ticks
    {
        return Err(invalid(
            "sustained performance final counts or window differs",
        ));
    }
    let actual = ActiveLoadSummary {
        target,
        before_step_below_target_ticks: before.iter().filter(|&&n| n < target).count(),
        after_step_below_target_ticks: after.iter().filter(|&&n| n < target).count(),
        before_step: sample_summary(&mut before)?,
        after_step: sample_summary(&mut after)?,
    };
    if result.active_load.as_ref() != Some(&actual)
        || measurement.intent_samples != before
        || measurement.active_samples != after
    {
        return Err(invalid(
            "sustained performance load differs between ticks, result and measurements",
        ));
    }
    let mut replacements = 0;
    for line in BufReader::new(File::open(directory.join("commands.jsonl"))?).lines() {
        let row: serde_json::Value = serde_json::from_str(&line?)?;
        let committed = row["committed"]
            .as_bool()
            .ok_or_else(|| invalid("missing sustained command result"))?;
        replacements += u64::from(committed);
    }
    if replacements != result.replacements {
        return Err(invalid("sustained performance recycling counts differ"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{measurement_fixture, write_round};
    use super::super::{
        compare_performance_runs, compare_runs, digest_file, line, sha256, write_json,
    };
    use super::*;
    use crate::{InitialVehicle, LifecycleCounts, RecyclingEntry, RecyclingPlan};
    use serde_json::json;
    use std::{fs, io::Write};

    // 封套校验夹具，无交通求值或正式性能证明；真实运行另用冻结的城市制品。
    fn packet(directory: &Path, execution: &str, workers: u32) {
        let target = 10_000;
        let cycle = 128;
        let window = Window::performance_for_cycle(cycle).unwrap();
        let samples = window.observation_ticks as usize;
        let mut measurement = measurement_fixture();
        measurement["workers"] = json!(workers);
        for field in [
            "command_samples_ns",
            "traffic_world_step_samples_ns",
            "observation_samples_ns",
        ] {
            measurement[field] = json!(vec![1; samples]);
        }
        measurement["intent_samples"] = json!(vec![target; samples]);
        let mut after = vec![target; samples];
        after[0] -= 1;
        measurement["active_samples"] = json!(after);
        write_round(directory, execution, measurement);
        let result_path = directory.join("result.json");
        let mut result: RunResult =
            serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
        let plan = ResolvedPlan {
            version: "urban-demand-v3".into(),
            case: "SUSTAINED-ACTIVE".into(),
            seed: 544,
            scale: "10k".into(),
            dt: 16,
            cycle_ticks: cycle,
            tiles: 10,
            individuals: target,
            manifest_digest: "fixture".into(),
            files: Default::default(),
            window: window.clone(),
            required_per_tile: Default::default(),
            initial_counts: LifecycleCounts {
                active: target,
                parked: 0,
                completed: 0,
            },
            max_attempts: 0,
            retry_ticks: 0,
            route_edges: [("route".into(), vec!["entry".into()])].into(),
            initial: (0..target)
                .map(|slot| InitialVehicle {
                    tile: slot / 1_000,
                    slot,
                    profile: "car".into(),
                    route: "route".into(),
                    occurrence: 0,
                    progress_mm: 7_000,
                    parking: None,
                    role: None,
                })
                .collect(),
            departures: vec![],
            leaves: vec![],
            arrivals: vec![],
            reservation_rejections: vec![],
            role_departures: vec![],
            boundary_windows: vec![],
            lifecycle_bursts: vec![],
            recycling: Some(RecyclingPlan {
                candidate_order: "rotate-entry-and-position".into(),
                retry_ticks: 8,
                attempts_per_boundary: 64,
                progress_mm: vec![7_000],
                entries_per_tile: vec![
                    vec![RecyclingEntry {
                        edge: "entry".into(),
                        routes: vec!["route".into()]
                    }];
                    10
                ],
            }),
        };
        let plan_bytes = toml::to_string_pretty(&plan).unwrap();
        fs::write(directory.join("resolved-plan.toml"), &plan_bytes).unwrap();
        result.case = plan.case.clone();
        result.window = window.clone();
        result.expected_ticks = window.end();
        result.completed_ticks = window.end();
        result.committed_world_tick = window.end();
        result.plan_digest = sha256(plan_bytes.as_bytes());
        result.initial_counts = (target as usize, 0, 0);
        result.final_counts = (target as usize - 1, 0, 1);
        result.checkpoints =
            std::iter::successors(Some(window.warm_up_ticks), |tick| tick.checked_add(cycle))
                .take_while(|&tick| tick <= window.end())
                .chain([0, window.end()])
                .map(|tick| (tick, "fixture".into()))
                .collect();
        let mut before = vec![u64::from(target); samples];
        let mut after = after.into_iter().map(u64::from).collect::<Vec<_>>();
        result.active_load = Some(ActiveLoadSummary {
            target: u64::from(target),
            before_step: sample_summary(&mut before).unwrap(),
            after_step: sample_summary(&mut after).unwrap(),
            before_step_below_target_ticks: 0,
            after_step_below_target_ticks: 1,
        });
        let mut ticks =
            std::io::BufWriter::new(File::create(directory.join("ticks.jsonl")).unwrap());
        for tick in 1..=window.end() {
            let dropped = tick <= window.warm_up_ticks || tick == window.end();
            line(
                &mut ticks,
                &TickRecord {
                    tick,
                    time_ms: tick * 16,
                    domain: "road_motor_vehicle".into(),
                    live: target as usize,
                    active: target as usize - usize::from(dropped),
                    parked: 0,
                    completed: usize::from(dropped),
                    intent: target as usize - usize::from(tick <= window.warm_up_ticks),
                    intent_basis: "exact_active_before_step".into(),
                    presented: 0,
                    aggregate_records: 0,
                    aggregate_equivalent: 0,
                    future_departures: 0,
                    pending_departures: 0,
                    exhausted_departures: 0,
                    command_cursor: 777,
                    event_cursor: 0,
                    state_digest: "fixture".into(),
                    event_digest: "fixture".into(),
                    commands_digest: "fixture".into(),
                },
            )
            .unwrap();
        }
        ticks.flush().unwrap();
        drop(ticks);
        for name in ["resolved-plan.toml", "ticks.jsonl"] {
            result
                .files
                .insert(name.into(), digest_file(&directory.join(name)).unwrap());
        }
        write_json(&result_path, &result).unwrap();
    }

    #[test]
    fn sustained_formal_packets_allow_warmup_and_post_step_deficits_only() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = ["a", "b", "c", "four"].map(|n| temp.path().join(n));
        for (i, dir) in dirs.iter().enumerate() {
            packet(dir, &format!("exec-{i}"), if i == 3 { 4 } else { 1 });
        }
        assert_eq!(
            compare_runs(&dirs[0], &dirs[3]).unwrap().status,
            "performance-match"
        );
        assert_eq!(
            compare_performance_runs([&dirs[0], &dirs[1], &dirs[2]])
                .unwrap()
                .case,
            "SUSTAINED-ACTIVE"
        );
        assert!(Window::performance_for_cycle(0).is_err());
        assert!(Window::performance_for_cycle(u64::MAX / 8).is_err());
    }

    #[test]
    fn sustained_formal_packets_reject_rehashed_inconsistent_load_and_plan_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = ["a", "b", "c", "four"].map(|n| temp.path().join(n));
        for (i, dir) in dirs.iter().enumerate() {
            packet(dir, &format!("exec-{i}"), if i == 3 { 4 } else { 1 });
        }
        for fault in [
            "tick-deficit",
            "measurement",
            "checkpoint",
            "recycling-count",
            "load-summary",
            "plan-window",
        ] {
            packet(&dirs[0], "exec-0", 1);
            let directory = &dirs[0];
            let path = directory.join("result.json");
            let mut result: RunResult = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match fault {
                "tick-deficit" => {
                    let rows = fs::read_to_string(directory.join("ticks.jsonl")).unwrap();
                    let mut output = std::io::BufWriter::new(
                        File::create(directory.join("ticks.jsonl")).unwrap(),
                    );
                    for text in rows.lines() {
                        let mut row: TickRecord = serde_json::from_str(text).unwrap();
                        if row.tick == result.window.warm_up_ticks + 1 {
                            row.intent -= 1;
                        }
                        line(&mut output, &row).unwrap();
                    }
                    output.flush().unwrap();
                    drop(output);
                }
                "measurement" => {
                    let mut value: toml::Value = toml::from_str(
                        &fs::read_to_string(directory.join("measurements.toml")).unwrap(),
                    )
                    .unwrap();
                    value["intent_samples"][0] = toml::Value::Integer(9_999);
                    fs::write(
                        directory.join("measurements.toml"),
                        toml::to_string(&value).unwrap(),
                    )
                    .unwrap();
                }
                "checkpoint" => {
                    result.checkpoints.remove(&result.window.warm_up_ticks);
                }
                "recycling-count" => {
                    result.replacements = 1;
                    result.births = 1;
                    result.removals = 1;
                }
                "load-summary" => {
                    result
                        .active_load
                        .as_mut()
                        .unwrap()
                        .after_step_below_target_ticks = 0;
                }
                "plan-window" => {
                    let mut plan =
                        ResolvedPlan::read(&directory.join("resolved-plan.toml")).unwrap();
                    plan.cycle_ticks += 1;
                    let text = toml::to_string_pretty(&plan).unwrap();
                    result.plan_digest = sha256(text.as_bytes());
                    fs::write(directory.join("resolved-plan.toml"), text).unwrap();
                }
                _ => unreachable!(),
            }
            for name in ["ticks.jsonl", "measurements.toml", "resolved-plan.toml"] {
                result
                    .files
                    .insert(name.into(), digest_file(&directory.join(name)).unwrap());
            }
            write_json(&path, &result).unwrap();
            assert!(
                compare_runs(directory, &dirs[3]).is_err(),
                "accepted {fault} across workers"
            );
            assert!(
                compare_performance_runs([directory, &dirs[1], &dirs[2]]).is_err(),
                "accepted {fault} across rounds"
            );
        }
    }
}
