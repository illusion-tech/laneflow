use std::{
    fs,
    num::NonZeroU32,
    path::Path,
    time::{Duration, Instant},
};

use bevy_app::App;
use bevy_ecs::{resource::Resource, schedule::IntoScheduleConfigs, world::World};
use bevy_time::Time;
use bevy_transform::components::Transform;
use laneflow_bevy::{
    LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_runtime::{ExecutionConfig, TickInput};
use laneflow_spatial::{FramePlacementToken, SpatialSession};
use laneflow_synth_generator::{Case, verify_directory};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Error, Plan, failure, install::Sample, require, write_json};

use super::{
    driver::Driver,
    frames::{self, FrameInput},
    presentation::Presentation,
    resources::{self, Allocation, AllocationStart},
};

#[derive(Serialize)]
struct TickSample {
    frame: u64,
    tick: u64,
    observation: bool,
    core_ns: u64,
    lifecycle_ns: u64,
    lifecycle_commands: usize,
    core_allocation: Option<Allocation>,
    lifecycle_allocation: Option<Allocation>,
    traffic_active_before_step: u32,
}

#[derive(Serialize)]
struct FrameSample {
    input: FrameInput,
    final_tick: u64,
    steps: u32,
    backlog_ns: u64,
    limit_reached: bool,
    lifecycle_commands: usize,
    selected: usize,
    sourced: usize,
    applied: usize,
    spatial_adapter_ns: u64,
    integrated_ns: u64,
    group: &'static str,
    population: [u32; 3],
}

#[derive(Resource)]
struct Timing {
    frame: u64,
    warm: u64,
    started: Instant,
    allocation: AllocationStart,
    lifecycle_ns: u64,
    lifecycle_allocation: Option<Allocation>,
    lifecycle_commands: usize,
    active_before: u32,
    ticks: Vec<TickSample>,
}

fn lifecycle(ecs: &mut World) {
    ecs.resource_scope(|ecs, mut driver: bevy_ecs::world::Mut<Driver>| {
        let before = driver.lifecycle_commands;
        let allocation = resources::start();
        let started = Instant::now();
        driver
            .lifecycle(ecs)
            .unwrap_or_else(|e| std::panic::panic_any(e));
        let elapsed = nanos(started);
        let allocation = resources::finish(allocation);
        let mut timing = ecs.resource_mut::<Timing>();
        timing.lifecycle_ns = elapsed;
        timing.lifecycle_allocation = allocation;
        timing.lifecycle_commands = driver.lifecycle_commands - before;
        timing.active_before = driver.counts[0];
    });
}

fn begin_step(ecs: &mut World) {
    let mut timing = ecs.resource_mut::<Timing>();
    timing.allocation = resources::start();
    timing.started = Instant::now();
}

fn end_step(ecs: &mut World) {
    let elapsed = nanos(ecs.resource::<Timing>().started);
    let allocation = resources::finish(ecs.resource::<Timing>().allocation);
    let session = ecs.resource::<LaneFlowSession>();
    if let Some(error) = session.last_error() {
        std::panic::panic_any(failure("Core step", error));
    }
    let tick = session.world().tick_index();
    let mut timing = ecs.resource_mut::<Timing>();
    let row = TickSample {
        frame: timing.frame,
        tick,
        observation: tick > timing.warm,
        core_ns: elapsed,
        lifecycle_ns: timing.lifecycle_ns,
        lifecycle_commands: timing.lifecycle_commands,
        core_allocation: allocation,
        lifecycle_allocation: timing.lifecycle_allocation,
        traffic_active_before_step: timing.active_before,
    };
    timing.ticks.push(row);
}

fn observe(ecs: &mut World) {
    ecs.resource_scope(|ecs, mut driver: bevy_ecs::world::Mut<Driver>| {
        let session = ecs.resource::<LaneFlowSession>();
        driver
            .observe(session.world(), session.frame_step_results().last())
            .unwrap_or_else(|e| std::panic::panic_any(e));
    });
}

fn nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 在新进程中执行一次完整窗口；oracle 必须来自同输入的独立语义运行。
///
/// # Errors
/// 输入、比例、提交、oracle 身份不符，输出已存在，I/O，安装、步进、表现或语义对照
/// 失败时返回错误。失败保留不完整回执；诊断构建的耗时不进入 latency 汇总。
pub fn run(
    input: &Path,
    oracle: &Path,
    implementation: &str,
    environment: &Path,
    percent: u8,
    round: u8,
    output: &Path,
) -> Result<Value, Error> {
    require(
        implementation.len() == 40
            && implementation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && (1..=3).contains(&round),
        || "提交或轮次非法".into(),
    )?;
    fs::create_dir(output)?;
    let mut status = json!({"format":"lf-synth-measurement-1","complete":false,"implementation_commit":implementation,
        "instrumented":cfg!(feature="allocation"),"round":round,"presentation_percent":percent});
    write_json(&output.join("report.json"), &status)?;
    let result = std::panic::catch_unwind(|| {
        run_inner(
            input,
            oracle,
            implementation,
            environment,
            percent,
            round,
            output,
        )
    })
    .unwrap_or_else(|panic| {
        let message = if let Some(e) = panic.downcast_ref::<Error>() {
            e.to_string()
        } else if let Some(e) = panic.downcast_ref::<String>() {
            e.clone()
        } else if let Some(e) = panic.downcast_ref::<&str>() {
            e.to_string()
        } else {
            "测量进程发生未分类 panic".into()
        };
        Err(Error::Invalid(message))
    });
    match result {
        Ok(report) => {
            write_json(&output.join("report.json"), &report)?;
            Ok(report)
        }
        Err(error) => {
            status["error"] = json!(error.to_string());
            write_json(&output.join("report.json"), &status)?;
            Err(error)
        }
    }
}

fn checkpoint(
    world: &laneflow_runtime::TrafficWorld,
    driver: &Driver,
    oracle: &Value,
) -> Result<Value, Error> {
    driver.validate_population(world)?;
    let tick = world.tick_index();
    let digest = crate::burst::state_digest(world)?;
    let expected = oracle["checkpoints"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["tick"] == tick))
        .ok_or_else(|| failure("oracle 缺少检查点", tick))?;
    require(expected["state_digest"] == digest, || {
        format!("状态与 oracle 不同 tick={tick}")
    })?;
    Ok(
        json!({"tick":tick,"time_ms":world.time_ms(),"state_digest":digest,"oracle_equal":true,
        "command_cursor":world.command_cursor(),"population":driver.counts}),
    )
}

fn run_inner(
    input: &Path,
    oracle_path: &Path,
    implementation: &str,
    environment: &Path,
    percent: u8,
    round: u8,
    output: &Path,
) -> Result<Value, Error> {
    let bundle = verify_directory(input)?;
    let plan = Plan::read(&bundle)?;
    let case = plan.identity.case;
    require(
        match case {
            Case::W2 => percent == 0,
            Case::W4 => {
                percent
                    == if plan.identity.scale == 10_000 {
                        100
                    } else {
                        10
                    }
            }
            _ => [1, 10, 50, 100].contains(&percent),
        },
        || "表现比例不符合 workload".into(),
    )?;
    let oracle: Value = serde_json::from_slice(&fs::read(oracle_path.join("report.json"))?)?;
    require(
        oracle["complete"] == true
            && oracle["input_manifest"] == serde_json::to_value(bundle.manifest())?
            && oracle["ordered_events_digest"].is_string(),
        || "oracle 未完成、缺少事件或输入身份不符".into(),
    )?;
    let environment: Value = serde_json::from_slice(&fs::read(environment)?)?;
    let inputs = frames::inputs(&plan);
    let selected = frames::selection(plan.identity.scale, percent);
    let inputs_bytes = serde_json::to_vec(&inputs)?;
    let selected_bytes = serde_json::to_vec(&selected)?;
    fs::write(output.join("frame-inputs.json"), &inputs_bytes)?;
    fs::write(output.join("selection.json"), &selected_bytes)?;
    write_json(&output.join("environment.json"), &environment)?;
    let binary = std::env::current_exe()?;
    let manifest = json!({"implementation_commit":implementation,"binary":binary,"binary_sha256":hash(&fs::read(&binary)?),
        "input_manifest":bundle.manifest(),"outer_frame_sha256":hash(&inputs_bytes),"selection_sha256":hash(&selected_bytes),
        "round":round,"presentation_percent":percent,"instrumented":cfg!(feature="allocation"),
        "fixed_delta_ms":plan.identity.fixed_delta_ms,"max_catch_up_steps":2,"workers":1,
        "timing":"continuous_frame; Core brackets Bevy Step set including scheduler boundary; snapshots/files/sorting between frames",
        "observation_extra_full_population_audit_inside_frame":false,"environment":environment,
        "oracle_report_sha256":hash(&fs::read(oracle_path.join("report.json"))?)});
    write_json(&output.join("manifest.json"), &manifest)?;
    let sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN))?;
    drop(bundle);
    let mut driver = Driver::new(&sample, &plan)?;
    driver.observe(&sample.world, None)?;
    let mut checkpoints = vec![checkpoint(&sample.world, &driver, &oracle)?];
    let mut memory = vec![
        json!({"phase":"installed","process":resources::process()?,"runtime":resources::runtime(&sample.world)}),
    ];
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let (ticks, frames, driver) = if case == Case::W2 {
        let mut world = sample.world;
        let mut ticks = Vec::with_capacity(total as usize);
        for tick in 1..=total {
            let allocation = resources::start();
            let started = Instant::now();
            let result = world
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure("Core-only step", e))?;
            let elapsed = nanos(started);
            let allocation = resources::finish(allocation);
            driver.observe(&world, Some(&result))?;
            ticks.push(TickSample {
                frame: tick - 1,
                tick,
                observation: tick > plan.warm_up_ticks,
                core_ns: elapsed,
                lifecycle_ns: 0,
                lifecycle_commands: 0,
                core_allocation: allocation,
                lifecycle_allocation: None,
                traffic_active_before_step: driver.counts[0],
            });
            if is_checkpoint(&plan, tick) {
                checkpoints.push(checkpoint(&world, &driver, &oracle)?);
                memory.push(json!({"tick":tick,"process":resources::process()?,"runtime":resources::runtime(&world)}));
                save_partial(output, &ticks, &[], &checkpoints)?;
                eprintln!("W2 tick={tick}/{total}");
            }
        }
        (ticks, Vec::new(), driver)
    } else {
        let mut app = create_app(sample, &plan, selected, driver)?;
        let mut frames = Vec::with_capacity(inputs.len());
        for input in inputs {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_nanos(input.delta_ns));
            app.world_mut().resource_mut::<Timing>().frame = input.index;
            app.world_mut().resource_mut::<Driver>().lifecycle_commands = 0;
            let started = Instant::now();
            app.update();
            let presentation_started = Instant::now();
            let applied = app.world_mut().resource_scope(
                |ecs, mut presentation: bevy_ecs::world::Mut<Presentation>| {
                    presentation.apply(ecs, FramePlacementToken::new(input.index + 1))
                },
            )?;
            let spatial_adapter_ns = nanos(presentation_started);
            let integrated_ns = nanos(started);
            let ecs = app.world();
            let session = ecs.resource::<LaneFlowSession>();
            if let Some(error) = session.last_error() {
                return Err(failure("frame", error));
            }
            let report = session.frame_report();
            let driver = ecs.resource::<Driver>();
            let group = group(
                &input,
                report.steps_run(),
                report.catch_up_limit_reached(),
                driver.lifecycle_commands,
            );
            frames.push(FrameSample {
                input,
                final_tick: session.world().tick_index(),
                steps: report.steps_run(),
                backlog_ns: report.backlog().as_nanos() as u64,
                limit_reached: report.catch_up_limit_reached(),
                lifecycle_commands: driver.lifecycle_commands,
                selected: ecs.resource::<Presentation>().selected.len(),
                sourced: applied,
                applied,
                spatial_adapter_ns,
                integrated_ns,
                group,
                population: driver.counts,
            });
            if input.boundary || input.burst.is_some() {
                let tick = session.world().tick_index();
                require(
                    report.backlog() == Duration::ZERO
                        && tick <= total
                        && session.world().time_ms()
                            == tick * u64::from(plan.identity.fixed_delta_ms),
                    || "边界时刻/backlog 不符".into(),
                )?;
                checkpoints.push(checkpoint(session.world(), driver, &oracle)?);
                memory.push(json!({"tick":tick,"process":resources::process()?,"runtime":resources::runtime(session.world()),
                    "presentation":ecs.resource::<Presentation>().resources()}));
                save_partial(
                    output,
                    &ecs.resource::<Timing>().ticks,
                    &frames,
                    &checkpoints,
                )?;
                eprintln!("{case:?} {percent}% tick={tick}/{total}");
            }
        }
        let ticks = app.world_mut().remove_resource::<Timing>().unwrap().ticks;
        let driver = app.world_mut().remove_resource::<Driver>().unwrap();
        (ticks, frames, driver)
    };
    require(
        ticks.len() as u64 == total
            && ticks
                .iter()
                .enumerate()
                .all(|(i, t)| t.tick == i as u64 + 1),
        || "Tick 数目或次序不符".into(),
    )?;
    driver.events.write(&output.join("events.bin"))?;
    require(
        fs::read(output.join("events.bin"))? == fs::read(oracle_path.join("events.bin"))?,
        || "有序事件与独立 oracle 不同".into(),
    )?;
    if matches!(case, Case::W3 | Case::W4) {
        fs::write(output.join("commands.bin"), &driver.receipts)?;
        require(
            driver.receipts == fs::read(oracle_path.join("commands.bin"))?,
            || "命令回执与独立 oracle 不同".into(),
        )?;
        let expected: Value = serde_json::from_slice(&fs::read(oracle_path.join("batches.json"))?)?;
        require(
            expected["batches"]
                .as_array()
                .is_some_and(|v| v.len() == driver.batches.len()),
            || "命令批次数不同".into(),
        )?;
        for (actual, expected) in driver
            .batches
            .iter()
            .zip(expected["batches"].as_array().unwrap())
        {
            for key in [
                "boundary",
                "command_count",
                "first_receipt",
                "input_digest",
                "cursor_before",
                "cursor_after",
            ] {
                require(actual[key] == expected[key], || {
                    format!("命令批次 {key} 不同")
                })?;
            }
        }
    }
    write_json(&output.join("batches.json"), &json!(driver.batches))?;
    write_json(&output.join("resources.json"), &json!(memory))?;
    save_partial(output, &ticks, &frames, &checkpoints)?;
    let statistics = statistics(&ticks, &frames, case);
    let files=["ticks.json","frames.json","events.bin","checkpoints.json","resources.json","batches.json"]
        .into_iter().map(|name| Ok((name.to_owned(),json!({"sha256":hash(&fs::read(output.join(name))?),"bytes":fs::metadata(output.join(name))?.len()}))))
        .collect::<Result<serde_json::Map<_,_>,Error>>()?;
    Ok(
        json!({"format":"lf-synth-measurement-1","complete":true,"implementation_commit":implementation,
        "instrumented":cfg!(feature="allocation"),"round":round,"presentation_percent":percent,"identity":plan.identity,
        "scope":"research_measurement_product_TBD_uncertified","manifest":manifest,
        "warm_up_ticks":plan.warm_up_ticks,"observation_ticks":plan.observation_ticks,"statistics":statistics,
        "ordered_events_equal":true,"ordered_events_digest":driver.events.digest(),"command_receipts_equal":true,
        "caller_commands":driver.command_count,"blocked_commands":driver.blocked_count,"checkpoints":checkpoints,
        "completion_records":driver.completion_records,"arrival_records":driver.arrival_records,"files":files}),
    )
}

fn is_checkpoint(plan: &Plan, tick: u64) -> bool {
    [
        plan.warm_up_ticks,
        plan.warm_up_ticks + 1_024,
        plan.warm_up_ticks + 2_048,
        plan.warm_up_ticks + 4_096,
        plan.warm_up_ticks + plan.observation_ticks,
    ]
    .contains(&tick)
}

fn group(input: &FrameInput, steps: u32, limit: bool, commands: usize) -> &'static str {
    if input.burst.is_some() {
        "synchronized_burst"
    } else if input.boundary {
        "boundary"
    } else if commands > 0 {
        "lifecycle_burst"
    } else if steps >= 2 || limit {
        "catch_up"
    } else if steps == 0 {
        "ordinary_zero_tick"
    } else {
        "ordinary_one_tick"
    }
}

fn save_partial(
    output: &Path,
    ticks: &[TickSample],
    frames: &[FrameSample],
    checkpoints: &[Value],
) -> Result<(), Error> {
    write_json(&output.join("ticks.json"), &json!(ticks))?;
    write_json(&output.join("frames.json"), &json!(frames))?;
    write_json(&output.join("checkpoints.json"), &json!(checkpoints))
}

fn statistics(ticks: &[TickSample], frames: &[FrameSample], case: Case) -> Value {
    if cfg!(feature = "allocation") {
        let aggregate = |lifecycle: bool| {
            let mut sum = Allocation::default();
            for tick in ticks.iter().filter(|t| t.observation) {
                if let Some(a) = if lifecycle {
                    tick.lifecycle_allocation
                } else {
                    tick.core_allocation
                } {
                    sum.allocations += a.allocations;
                    sum.reallocations += a.reallocations;
                    sum.deallocations += a.deallocations;
                    sum.bytes_allocated += a.bytes_allocated;
                    sum.bytes_deallocated += a.bytes_deallocated;
                    sum.bytes_reallocated += a.bytes_reallocated;
                }
            }
            sum
        };
        return json!({"latency_excluded":true,"core_allocations":aggregate(false),"lifecycle_allocations":aggregate(true)});
    }
    if case == Case::W4 {
        let raw: Vec<_> = frames
            .iter()
            .filter(|f| f.input.observation)
            .map(|f| {
                json!({"burst":f.input.burst,
            "core_ns":ticks.iter().find(|t|t.frame==f.input.index).unwrap().core_ns,
            "spatial_adapter_ns":f.spatial_adapter_ns,"integrated_ns":f.integrated_ns})
            })
            .collect();
        let mut max = serde_json::Map::new();
        for key in ["core_ns", "spatial_adapter_ns", "integrated_ns"] {
            max.insert(
                key.into(),
                json!(raw.iter().map(|r| r[key].as_u64().unwrap()).max()),
            );
        }
        return json!({"raw":raw,"burst_max":max});
    }
    let mut groups = serde_json::Map::new();
    for group in [
        "ordinary_zero_tick",
        "ordinary_one_tick",
        "catch_up",
        "lifecycle_burst",
        "boundary",
    ] {
        let rows: Vec<_> = frames
            .iter()
            .filter(|f| f.input.observation && f.group == group)
            .collect();
        if !rows.is_empty() {
            groups.insert(group.into(),json!({"spatial_adapter":frames::distribution(rows.iter().map(|f|f.spatial_adapter_ns)),
            "integrated":frames::distribution(rows.iter().map(|f|f.integrated_ns))}));
        }
    }
    json!({"core":frames::distribution(ticks.iter().filter(|t|t.observation).map(|t|t.core_ns)),"frame_groups":groups})
}

fn create_app(
    sample: Sample,
    plan: &Plan,
    selected: Vec<usize>,
    mut driver: Driver,
) -> Result<App, Error> {
    let total = plan.warm_up_ticks + plan.observation_ticks;
    let spatial = SpatialSession::bind(sample.world.revision())
        .map_err(|e| failure("Spatial bind", e))?
        .ok_or_else(|| failure("Spatial", "missing component"))?;
    let mut session = LaneFlowSession::new(
        sample.world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(2).unwrap()),
    )
    .map_err(|e| failure("Session", e))?;
    let mut app = App::new();
    for &rank in &selected {
        let entity = app.world_mut().spawn(Transform::IDENTITY).id();
        session
            .bind_vehicle_entity(driver.handles[rank], entity)
            .map_err(|e| failure("initial binding", e))?;
        driver.entities[rank] = Some(entity);
    }
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(Time::<()>::default())
        .insert_resource(session)
        .insert_resource(driver)
        .insert_resource(Presentation::new(selected))
        .insert_resource(Timing {
            frame: 0,
            warm: plan.warm_up_ticks,
            started: Instant::now(),
            allocation: resources::start(),
            lifecycle_ns: 0,
            lifecycle_allocation: None,
            lifecycle_commands: 0,
            active_before: 0,
            ticks: Vec::with_capacity(total as usize),
        })
        .add_systems(LaneFlowFixed, lifecycle.in_set(LaneFlowFixedSet::Lifecycle))
        .add_systems(
            LaneFlowFixed,
            begin_step
                .after(LaneFlowFixedSet::Lifecycle)
                .before(LaneFlowFixedSet::Step),
        )
        .add_systems(
            LaneFlowFixed,
            end_step
                .after(LaneFlowFixedSet::Step)
                .before(LaneFlowFixedSet::Observe),
        )
        .add_systems(LaneFlowFixed, observe.in_set(LaneFlowFixedSet::Observe));
    Ok(app)
}

/// 单独执行零/一/两拍、追赶上限与两 quantum backlog 恢复序列。
///
/// # Errors
/// 输入、安装、调度、表现、独立重放或输出 I/O 失败时返回错误。
pub fn guardrail(input: &Path, output: &Path) -> Result<Value, Error> {
    let bundle = verify_directory(input)?;
    let plan = Plan::read(&bundle)?;
    require(plan.identity.case == Case::W1, || {
        "调度 guardrail 使用 W1".into()
    })?;
    fs::create_dir(output)?;
    write_json(&output.join("report.json"), &json!({"complete":false}))?;
    let sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN))?;
    let mut driver = Driver::new(&sample, &plan)?;
    driver.observe(&sample.world, None)?;
    let mut oracle = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN))?.world;
    let mut oracle_events = super::events::Recorder::new(16, plan.identity.scale);
    oracle_events.record(&oracle, &driver.ranks, None)?;
    let mut app = create_app(
        sample,
        &plan,
        frames::selection(plan.identity.scale, 1),
        driver,
    )?;
    let q = u64::from(plan.identity.fixed_delta_ms) * 1_000_000;
    let deltas = [0, q / 2, q / 2, q, 2 * q, 4 * q, 0, q / 2, q / 2];
    let expected_steps = [0, 0, 1, 1, 2, 2, 2, 0, 1];
    let expected_backlog = [0, q / 2, 0, 0, 0, 2 * q, 0, q / 2, 0];
    let mut rows = Vec::new();
    for (index, delta) in deltas.into_iter().enumerate() {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_nanos(delta));
        app.world_mut().resource_mut::<Timing>().frame = index as u64;
        app.update();
        app.world_mut().resource_scope(
            |ecs, mut presentation: bevy_ecs::world::Mut<Presentation>| {
                presentation.apply(ecs, FramePlacementToken::new(index as u64 + 1))
            },
        )?;
        let session = app.world().resource::<LaneFlowSession>();
        let report = session.frame_report();
        require(
            report.steps_run() == expected_steps[index]
                && report.backlog().as_nanos() == u128::from(expected_backlog[index])
                && report.catch_up_limit_reached() == (index == 5),
            || format!("调度序列不符 frame={index}"),
        )?;
        for _ in 0..report.steps_run() {
            let result = oracle
                .step(TickInput::new(u64::from(plan.identity.fixed_delta_ms)))
                .map_err(|e| failure("guardrail oracle", e))?;
            oracle_events.record(
                &oracle,
                &app.world().resource::<Driver>().ranks,
                Some(&result),
            )?;
        }
        let actual = crate::burst::state_digest(session.world())?;
        require(actual == crate::burst::state_digest(&oracle)?, || {
            "追赶状态与逐拍重放不同".into()
        })?;
        rows.push(json!({"frame":index,"delta_ns":delta,"steps":report.steps_run(),"tick":session.world().tick_index(),
            "backlog_ns":report.backlog().as_nanos() as u64,"limit_reached":report.catch_up_limit_reached(),"state_digest":actual,"replay_equal":true}));
    }
    let driver = app.world().resource::<Driver>();
    driver.events.write(&output.join("events.bin"))?;
    oracle_events.write(&output.join("oracle-events.bin"))?;
    require(
        fs::read(output.join("events.bin"))? == fs::read(output.join("oracle-events.bin"))?,
        || "追赶丢失中间拍事件".into(),
    )?;
    let report = json!({"format":"lf-synth-scheduler-guardrail-1","complete":true,"identity":plan.identity,
        "scope":"independent_semantic_guardrail_not_latency","rows":rows,"two_quantum_backlog_recovery_frames":1,"ordered_events_equal":true});
    write_json(&output.join("report.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_synth_generator::{Identity, Provenance, generate};

    fn fixture(case: Case, dt: u32) -> (Sample, Plan) {
        let bundle = generate(
            Identity::new(10_000, case, dt).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        (
            Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap(),
            Plan::read(&bundle).unwrap(),
        )
    }

    #[test]
    fn actual_bevy_guardrail_keeps_every_intermediate_tick() {
        for dt in [16, 33] {
            let (sample, plan) = fixture(Case::W1, dt);
            let mut driver = Driver::new(&sample, &plan).unwrap();
            driver.observe(&sample.world, None).unwrap();
            let mut app = create_app(sample, &plan, frames::selection(10_000, 1), driver).unwrap();
            let q = u64::from(dt) * 1_000_000;
            for (index, (delta, steps, backlog)) in [
                (0, 0, 0),
                (q / 2, 0, q / 2),
                (q / 2, 1, 0),
                (4 * q, 2, 2 * q),
                (0, 2, 0),
            ]
            .into_iter()
            .enumerate()
            {
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(Duration::from_nanos(delta));
                app.world_mut().resource_mut::<Timing>().frame = index as u64;
                app.update();
                let report = app.world().resource::<LaneFlowSession>().frame_report();
                assert_eq!(
                    (report.steps_run(), report.backlog().as_nanos()),
                    (steps, u128::from(backlog))
                );
                assert_eq!(report.catch_up_limit_reached(), index == 3);
            }
            let ticks = &app.world().resource::<Timing>().ticks;
            assert_eq!(
                ticks.iter().map(|t| t.tick).collect::<Vec<_>>(),
                [1, 2, 3, 4, 5]
            );
            assert_eq!(
                ticks.iter().map(|t| t.frame).collect::<Vec<_>>(),
                [2, 3, 3, 4, 4]
            );
            app.world_mut()
                .resource_scope(|ecs, mut driver: bevy_ecs::world::Mut<Driver>| {
                    assert!(
                        driver
                            .observe(ecs.resource::<LaneFlowSession>().world(), None)
                            .is_err()
                    );
                });
        }
    }

    #[test]
    fn late_missing_transform_keeps_every_earlier_transform_unchanged() {
        let (sample, plan) = fixture(Case::W1, 16);
        let mut driver = Driver::new(&sample, &plan).unwrap();
        driver.observe(&sample.world, None).unwrap();
        let selected = frames::selection(10_000, 1);
        let mut app = create_app(sample, &plan, selected.clone(), driver).unwrap();
        app.world_mut()
            .resource_scope(
                |ecs, mut presentation: bevy_ecs::world::Mut<Presentation>| {
                    presentation.apply(ecs, FramePlacementToken::new(1))
                },
            )
            .unwrap();
        let entities: Vec<_> = selected
            .iter()
            .map(|&r| app.world().resource::<Driver>().entities[r].unwrap())
            .collect();
        let before: Vec<_> = entities
            .iter()
            .map(|&e| *app.world().get::<Transform>(e).unwrap())
            .collect();
        app.world_mut()
            .entity_mut(*entities.last().unwrap())
            .remove::<Transform>();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(16));
        app.update();
        assert!(
            app.world_mut()
                .resource_scope(|ecs, mut p: bevy_ecs::world::Mut<Presentation>| p
                    .apply(ecs, FramePlacementToken::new(2)))
                .is_err()
        );
        for (&entity, old) in entities[..entities.len() - 1].iter().zip(&before) {
            assert_eq!(*app.world().get::<Transform>(entity).unwrap(), *old);
        }
        app.world_mut()
            .entity_mut(*entities.last().unwrap())
            .insert(Transform::IDENTITY);
        app.world_mut()
            .resource_scope(|ecs, mut p: bevy_ecs::world::Mut<Presentation>| {
                p.apply(ecs, FramePlacementToken::new(3))
            })
            .unwrap();
        assert!(
            entities
                .iter()
                .zip(&before)
                .any(|(&e, old)| app.world().get::<Transform>(e).unwrap() != old)
        );
    }

    #[test]
    fn frame_classification_keeps_overlapping_flags_and_budget_groups_disjoint() {
        let mut input = FrameInput {
            index: 0,
            delta_ns: 1,
            boundary: false,
            observation: true,
            burst: None,
        };
        assert_eq!(group(&input, 0, false, 0), "ordinary_zero_tick");
        assert_eq!(group(&input, 1, false, 0), "ordinary_one_tick");
        assert_eq!(group(&input, 2, true, 0), "catch_up");
        assert_eq!(group(&input, 2, true, 1), "lifecycle_burst");
        input.boundary = true;
        assert_eq!(group(&input, 2, true, 1), "boundary");
        input.burst = Some(0);
        assert_eq!(group(&input, 1, false, 1), "synchronized_burst");
    }

    #[test]
    fn logical_selection_and_entity_survive_despawn_completion_and_typed_replacement() {
        let (sample, mut plan) = fixture(Case::W1, 16);
        let command = |operation: &str, progress| {
            json!({"operation":operation,"logical_rank":0,"route_rank":0,
            "edge_occurrence":1,"progress_mm":progress,"speed_mm_per_s":0})
        };
        plan.scheduled = vec![
            json!({"boundary":0,"commands":[command("despawn",0),command("spawn",5_130_000)]}),
            json!({"boundary":1,"commands":[command("atomic_replace",4_000_000)]}),
        ];
        let mut driver = Driver::new(&sample, &plan).unwrap();
        driver.observe(&sample.world, None).unwrap();
        let selected = frames::selection(10_000, 100);
        let mut app = create_app(sample, &plan, selected.clone(), driver).unwrap();
        let entity = app.world().resource::<Driver>().entities[0].unwrap();
        let first = app.world().resource::<Driver>().handles[0];
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(16));
        app.update();
        let completed = app.world().resource::<Driver>().handles[0];
        assert_ne!(first, completed);
        assert_eq!(app.world().resource::<Driver>().counts[2], 1);
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .vehicle_entity(completed),
            Some(entity)
        );
        let count = app
            .world_mut()
            .resource_scope(|ecs, mut p: bevy_ecs::world::Mut<Presentation>| {
                p.apply(ecs, FramePlacementToken::new(1))
            })
            .unwrap();
        assert_eq!(count, 9_999);
        assert_eq!(app.world().resource::<Presentation>().selected, selected);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(16));
        app.update();
        let replaced = app.world().resource::<Driver>().handles[0];
        assert_ne!(completed, replaced);
        assert_eq!(
            app.world()
                .resource::<LaneFlowSession>()
                .vehicle_entity(replaced),
            Some(entity)
        );
        let count = app
            .world_mut()
            .resource_scope(|ecs, mut p: bevy_ecs::world::Mut<Presentation>| {
                p.apply(ecs, FramePlacementToken::new(2))
            })
            .unwrap();
        assert_eq!(count, 10_000);
        assert_eq!(app.world().resource::<Presentation>().selected, selected);
        app.world()
            .resource::<Driver>()
            .validate_population(app.world().resource::<LaneFlowSession>().world())
            .unwrap();
    }
}
