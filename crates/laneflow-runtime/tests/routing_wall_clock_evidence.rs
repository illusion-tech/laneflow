//! #303 Routing G2 具名工作负载的 wall-clock 描述性证据（未插桩）。
//!
//! 全局计数分配器会污染墙钟，因此本二进制不安装分配器；分配/retained
//! 证据见 `routing_budget_evidence`。计时只覆盖被测调用，输入构造、结果
//! 析构以及候选 route 的移除均在计时区间外。
//!
//! 本测试默认 ignored，CI 只编译。正式取证使用 release、单测试线程，
//! 并以三个 fresh process 独立运行；这些数字只描述具名初始 case，不是
//! LF-SYNTH W1-W4 Product Pass。
//!
//! 附加 LF-SYNTH-v2 正式负载入口（#1267）：`LF-ROUTING-CANDIDATE-LFSYNTH-v1`
//! 消费正式身份输入目录（环境变量门控），供 #1279 在冻结验收提交后零改码
//! 运行；除 §7.1 同款操作外新增干扰配对（显式调用后紧跟被计时 tick）。

use std::hint::black_box;
use std::time::Instant;

use laneflow_runtime::{CandidateRouteInput, ObservationExportMode, TickInput};

mod support {
    /// 共享的 Routing G2 证据 fixture 与具名工作负载常量。
    pub mod routing_evidence;
}

use support::routing_evidence::{
    CandidateAdmissionFixture, DELTA_MS, EDGE_COUNT, LONG_ROUTE_EDGE_COUNT, ReceiverLimits,
    TYPICAL_ROUTE_EDGE_COUNT, WORKLOAD_ID, build_fixture, build_lf_synth_fixture,
    fixed_input_sequence_digest, receive_cost_snapshot,
};

const SAMPLE_WARMUP: usize = 3;
const SAMPLE_COUNT: usize = 21;
const FIXED_INPUT_SEQUENCE: &str = concat!(
    "fixture:install;step(4ms)x64;observation-full(all);",
    "receive-cost(entries=4096,bytes=98304);open-admission;",
    "measure-each(warmup=3,samples=21):step(4ms);observation-full;",
    "observation-delta-zero;receive-cost;candidate-register-remove(edges=1);",
    "candidate-register-remove(edges=128);candidate-register-remove(edges=4096);step(4ms);",
);

#[derive(Debug)]
struct ClockStats {
    raw_ns: Vec<u128>,
    median_ns: u128,
    mad_ns: u128,
    p95_ns: u128,
    p99_ns: u128,
    max_ns: u128,
}

impl ClockStats {
    fn from_raw(raw_ns: Vec<u128>) -> Self {
        assert_eq!(raw_ns.len(), SAMPLE_COUNT);
        let mut sorted = raw_ns.clone();
        sorted.sort_unstable();
        let median_ns = percentile_nearest_rank(&sorted, 50);
        let mut deviations: Vec<_> = sorted
            .iter()
            .map(|value| value.abs_diff(median_ns))
            .collect();
        deviations.sort_unstable();
        Self {
            median_ns,
            mad_ns: percentile_nearest_rank(&deviations, 50),
            p95_ns: percentile_nearest_rank(&sorted, 95),
            p99_ns: percentile_nearest_rank(&sorted, 99),
            max_ns: *sorted.last().expect("non-empty samples"),
            raw_ns,
        }
    }
}

fn percentile_nearest_rank(sorted: &[u128], percentile: usize) -> u128 {
    assert!(!sorted.is_empty());
    assert!((1..=100).contains(&percentile));
    let rank = sorted
        .len()
        .checked_mul(percentile)
        .and_then(|value| value.checked_add(99))
        .expect("percentile rank")
        / 100;
    sorted[rank.saturating_sub(1)]
}

fn measure<T>(mut operation: impl FnMut() -> T) -> ClockStats {
    measure_validated(&mut operation, |_| {})
}

fn measure_validated<T>(
    mut operation: impl FnMut() -> T,
    mut validate: impl FnMut(&T),
) -> ClockStats {
    let mut raw_ns = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..(SAMPLE_WARMUP + SAMPLE_COUNT) {
        let started = Instant::now();
        let output = operation();
        let elapsed = started.elapsed().as_nanos();
        validate(&output);
        black_box(&output);
        drop(output);
        if index >= SAMPLE_WARMUP {
            raw_ns.push(elapsed);
        }
    }
    ClockStats::from_raw(raw_ns)
}

fn print_stats(prefix: &str, name: &str, stats: &ClockStats) {
    let raw = stats
        .raw_ns
        .iter()
        .map(u128::to_string)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{prefix} operation={name} samples={} median_ns={} mad_ns={} p95_ns={} p99_ns={} max_ns={} raw_ns={raw}",
        stats.raw_ns.len(),
        stats.median_ns,
        stats.mad_ns,
        stats.p95_ns,
        stats.p99_ns,
        stats.max_ns,
    );
}

/// 预构样本输入（不可变阶段），再进入计时循环（可变阶段），构造成本
/// 不进入任何计时区间。
fn prebuilt_inputs(mut make: impl FnMut() -> CandidateRouteInput) -> Vec<CandidateRouteInput> {
    (0..(SAMPLE_WARMUP + SAMPLE_COUNT))
        .map(|_| make())
        .collect()
}

#[test]
#[ignore = "manual release wall-clock evidence; CI 不当 Routing 产品基线"]
fn routing_g2_wall_clock_evidence() {
    let mut fixture = build_fixture();
    assert_eq!(fixture.initial_full.entry_count(), EDGE_COUNT as u64);

    let steady_before = measure(|| {
        fixture
            .world
            .step(TickInput::new(DELTA_MS))
            .expect("steady tick before")
    });

    let observation_full = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Full,
                )
                .expect("full observation")
        },
        |batch| assert_eq!(batch.entry_count(), EDGE_COUNT as u64),
    );

    let observation_delta_zero = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Delta,
                )
                .expect("zero-change observation")
        },
        |batch| assert_eq!(batch.entry_count(), 0),
    );

    let receiver = measure(|| {
        receive_cost_snapshot(
            fixture.cost_binding,
            &fixture.cost_payload,
            ReceiverLimits::EXACT_WORKLOAD,
        )
        .expect("receive cost snapshot")
    });

    let candidate_one = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_input(1)),
        &mut fixture,
        1,
    );
    let candidate_typical = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_input(TYPICAL_ROUTE_EDGE_COUNT)),
        &mut fixture,
        TYPICAL_ROUTE_EDGE_COUNT,
    );
    let candidate_long = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_input(LONG_ROUTE_EDGE_COUNT)),
        &mut fixture,
        LONG_ROUTE_EDGE_COUNT,
    );

    let steady_after = measure(|| {
        fixture
            .world
            .step(TickInput::new(DELTA_MS))
            .expect("steady tick after")
    });

    let config = fixture.world.config();
    let observation_set = fixture.cost_binding.observation_set();
    let cost_model = fixture.cost_binding.cost_model();
    println!(
        "routing-g2-wall-clock workload_id={WORKLOAD_ID} seed={} lfca_exact_bytes={} artifact_digest={:x} topology_network_revision={:x} workload_manifest_digest={:x} state_digest={:x} selection_digest={:x} fixed_input_sequence_digest={:x} edge_count={} world_config_vehicle_capacity={} world_config_route_capacity={} world_config_route_edge_occurrence_capacity={} execution_worker_count={} world_config_fixed_delta_ms={} binding_version={} observation_tick={} observation_state_sequence={} observation_set_digest={:x} cost_model_id={:x} cost_model_version={} valid_through_tick={} snapshot_sha256={:x} cost_entry_count={} cost_exact_bytes={} receiver_limits_entries={} receiver_limits_bytes={} sample_warmup={} samples={} classification=descriptive-initial-not-product-pass",
        support::routing_evidence::WORKLOAD_SEED,
        fixture.lfca_exact_bytes,
        fixture.artifact_digest,
        fixture.network_revision.as_digest(),
        fixture.workload_manifest_digest,
        fixture.state_digest,
        fixture.initial_full.selection_digest(),
        fixed_input_sequence_digest(FIXED_INPUT_SEQUENCE),
        EDGE_COUNT,
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        fixture.world.execution_config().worker_count(),
        config.fixed_delta_time_ms(),
        fixture.cost_binding.binding_version(),
        observation_set.observation_tick(),
        observation_set.observation_state_sequence().get(),
        observation_set.digest(),
        cost_model.model_id(),
        cost_model.model_version(),
        fixture.cost_binding.valid_through_tick(),
        fixture.cost_binding.snapshot_sha256(),
        fixture.cost_binding.entry_count(),
        fixture.cost_binding.exact_byte_length(),
        ReceiverLimits::EXACT_WORKLOAD.max_entry_count,
        ReceiverLimits::EXACT_WORKLOAD.max_exact_byte_length,
        SAMPLE_WARMUP,
        SAMPLE_COUNT,
    );
    print_stats(
        "routing-g2-wall-clock",
        "steady_tick_before",
        &steady_before,
    );
    print_stats(
        "routing-g2-wall-clock",
        "observation_full",
        &observation_full,
    );
    print_stats(
        "routing-g2-wall-clock",
        "observation_delta_zero",
        &observation_delta_zero,
    );
    print_stats("routing-g2-wall-clock", "receiver", &receiver);
    print_stats(
        "routing-g2-wall-clock",
        "candidate_register_1",
        &candidate_one,
    );
    print_stats(
        "routing-g2-wall-clock",
        "candidate_register_128",
        &candidate_typical,
    );
    print_stats(
        "routing-g2-wall-clock",
        "candidate_register_4096",
        &candidate_long,
    );
    print_stats("routing-g2-wall-clock", "steady_tick_after", &steady_after);
}

fn measure_candidate(
    inputs: Vec<CandidateRouteInput>,
    fixture: &mut impl CandidateAdmissionFixture,
    edge_count: usize,
) -> ClockStats {
    assert_eq!(inputs.len(), SAMPLE_WARMUP + SAMPLE_COUNT);
    let mut raw_ns = Vec::with_capacity(SAMPLE_COUNT);
    for (index, input) in inputs.into_iter().enumerate() {
        let started = Instant::now();
        let route = fixture
            .register_candidate(input)
            .expect("register candidate route");
        let elapsed = started.elapsed().as_nanos();
        black_box(route);
        assert_eq!(
            fixture.world_mut().route_edges(route).map(<[_]>::len),
            Some(edge_count)
        );
        fixture
            .world_mut()
            .remove_route(route)
            .expect("remove measured route");
        if index >= SAMPLE_WARMUP {
            raw_ns.push(elapsed);
        }
    }
    ClockStats::from_raw(raw_ns)
}

// ===== LF-SYNTH-v2 正式负载（#1267；计时入口供 #1279 零改码正式运行） =====
//
// 协议与 §7.1 对齐：release、未插桩（无计数分配器）、每操作 3 warm-up +
// 21 measured samples、3 个 fresh-process rounds 由 #1279 编排（每身份每
// 轮一个进程）。本入口单进程单身份（环境变量选择）。除 §7.1 同款操作
// 外，为「墙钟与 tick 间隔干扰同时报告」新增干扰配对：每个显式调用后
// 紧跟一次计时 tick（candidate 配对保持路线存活至该 tick 之后）。
// 候选边数轴受输入族拓扑约束（无环短链，典型 2 边、最长 5 边），长轴
// 扩展性由 §7.1 的 4096 边 G2-LINEAR case 承担。

const LF_SYNTH_FIXED_INPUT_SEQUENCE: &str = concat!(
    "formal-identity:verify-directory;install;register-state-routes;spawn-vehicles;",
    "step(delta)x64;observation-full(all);receive-cost(all-edges);open-admission;",
    "measure-each(warmup=3,samples=21):step(delta);observation-full;",
    "observation-delta-zero;receive-cost;candidate-register-remove(edges=1);",
    "candidate-register-remove(edges=typical);candidate-register-remove(edges=long);",
    "interference-pairs(op-then-timed-step):observation-full;receive-cost;",
    "candidate-register-timed-step-remove(edges=1);candidate-register-timed-step-remove(edges=typical);",
    "candidate-register-timed-step-remove(edges=long);step(delta);",
);

fn measure_interference_pair<T>(
    fixture: &mut support::routing_evidence::LfSynthFixture,
    operation: impl Fn(&mut support::routing_evidence::LfSynthFixture) -> T,
    validate: impl Fn(&T),
) -> (ClockStats, ClockStats) {
    let delta_ms = fixture.fixed_delta_ms;
    let mut op_ns = Vec::with_capacity(SAMPLE_COUNT);
    let mut step_ns = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..(SAMPLE_WARMUP + SAMPLE_COUNT) {
        let started = Instant::now();
        let output = operation(fixture);
        let op_elapsed = started.elapsed().as_nanos();
        validate(&output);
        black_box(&output);
        drop(output);
        let stepped = Instant::now();
        fixture
            .world_mut()
            .step(TickInput::new(delta_ms))
            .expect("timed step after explicit call");
        let step_elapsed = stepped.elapsed().as_nanos();
        if index >= SAMPLE_WARMUP {
            op_ns.push(op_elapsed);
            step_ns.push(step_elapsed);
        }
    }
    (ClockStats::from_raw(op_ns), ClockStats::from_raw(step_ns))
}

fn measure_candidate_interference_pair(
    inputs: Vec<CandidateRouteInput>,
    fixture: &mut support::routing_evidence::LfSynthFixture,
    edge_count: usize,
) -> (ClockStats, ClockStats) {
    assert_eq!(inputs.len(), SAMPLE_WARMUP + SAMPLE_COUNT);
    let delta_ms = fixture.fixed_delta_ms;
    let mut op_ns = Vec::with_capacity(SAMPLE_COUNT);
    let mut step_ns = Vec::with_capacity(SAMPLE_COUNT);
    for (index, input) in inputs.into_iter().enumerate() {
        let started = Instant::now();
        let route = fixture
            .register_candidate(input)
            .expect("register candidate route");
        let op_elapsed = started.elapsed().as_nanos();
        // 路线保持存活越过被计时的下一拍，显式调用的跨拍干扰才可测。
        let stepped = Instant::now();
        fixture
            .world_mut()
            .step(TickInput::new(delta_ms))
            .expect("timed step after candidate register");
        let step_elapsed = stepped.elapsed().as_nanos();
        assert_eq!(
            fixture.world_mut().route_edges(route).map(<[_]>::len),
            Some(edge_count)
        );
        black_box(route);
        fixture
            .world_mut()
            .remove_route(route)
            .expect("remove paired candidate");
        if index >= SAMPLE_WARMUP {
            op_ns.push(op_elapsed);
            step_ns.push(step_elapsed);
        }
    }
    (ClockStats::from_raw(op_ns), ClockStats::from_raw(step_ns))
}

#[test]
#[ignore = "manual release wall-clock evidence for LF-SYNTH candidate admission; 正式数字由 #1279 在安静窗口采信"]
fn routing_candidate_lf_synth_wall_clock_evidence() {
    let mut fixture = build_lf_synth_fixture();
    let delta_ms = fixture.fixed_delta_ms;
    let edge_count = fixture.edge_count as u64;
    let typical_edges = fixture.typical_route.len();
    let long_edges = fixture.long_route.len();
    assert_eq!(fixture.initial_full.entry_count(), edge_count);

    let steady_before = measure(|| {
        fixture
            .world_mut()
            .step(TickInput::new(delta_ms))
            .expect("steady tick before")
    });

    let observation_full = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Full,
                )
                .expect("full observation")
        },
        |batch| assert_eq!(batch.entry_count(), edge_count),
    );

    let observation_delta_zero = measure_validated(
        || {
            fixture
                .world
                .export_observation(
                    &mut fixture.observation_session,
                    ObservationExportMode::Delta,
                )
                .expect("zero-change observation")
        },
        |batch| assert_eq!(batch.entry_count(), 0),
    );

    let receiver = measure(|| {
        receive_cost_snapshot(
            fixture.cost_binding(),
            fixture.cost_payload(),
            fixture.receiver_limits(),
        )
        .expect("receive cost snapshot")
    });

    let candidate_one = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_one_input()),
        &mut fixture,
        1,
    );
    let candidate_typical = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_typical_input()),
        &mut fixture,
        typical_edges,
    );
    let candidate_long = measure_candidate(
        prebuilt_inputs(|| fixture.candidate_long_input()),
        &mut fixture,
        long_edges,
    );

    let (pair_full, step_after_full) = measure_interference_pair(
        &mut fixture,
        |fx| {
            fx.world
                .export_observation(&mut fx.observation_session, ObservationExportMode::Full)
                .expect("paired full observation")
        },
        |batch| assert_eq!(batch.entry_count(), edge_count),
    );
    let (pair_receiver, step_after_receiver) = measure_interference_pair(
        &mut fixture,
        |fx| {
            receive_cost_snapshot(fx.cost_binding, &fx.cost_payload, fx.receiver_limits())
                .expect("paired receive cost snapshot")
        },
        |binding| {
            black_box(binding);
        },
    );
    let (pair_one, step_after_one) = measure_candidate_interference_pair(
        prebuilt_inputs(|| fixture.candidate_one_input()),
        &mut fixture,
        1,
    );
    let (pair_typical, step_after_typical) = measure_candidate_interference_pair(
        prebuilt_inputs(|| fixture.candidate_typical_input()),
        &mut fixture,
        typical_edges,
    );
    let (pair_long, step_after_long) = measure_candidate_interference_pair(
        prebuilt_inputs(|| fixture.candidate_long_input()),
        &mut fixture,
        long_edges,
    );

    let steady_after = measure(|| {
        fixture
            .world_mut()
            .step(TickInput::new(delta_ms))
            .expect("steady tick after")
    });

    let config = fixture.world.config();
    let observation_set = fixture.cost_binding().observation_set();
    let cost_model = fixture.cost_binding().cost_model();
    let limits = fixture.receiver_limits();
    println!(
        "routing-lfsynth-wall-clock workload_id={} identity={} manifest_digest={:x} network_revision={:x} workload_manifest_digest={:x} state_digest={:x} selection_digest={:x} fixed_input_sequence_digest={:x} edge_count={} state_routes={} state_route_edges={} vehicles={} active={} parked={} typical_edges={} long_edges={} world_config_vehicle_capacity={} world_config_route_capacity={} world_config_route_edge_occurrence_capacity={} execution_worker_count={} world_config_fixed_delta_ms={} binding_version={} observation_tick={} observation_state_sequence={} observation_set_digest={:x} cost_model_id={:x} cost_model_version={} valid_through_tick={} snapshot_sha256={:x} cost_entry_count={} cost_exact_bytes={} receiver_limits_entries={} receiver_limits_bytes={} sample_warmup={} samples={} classification=descriptive-initial-not-product-pass",
        fixture.workload_id,
        fixture.identity_name,
        fixture.manifest_digest,
        fixture.network_revision.as_digest(),
        fixture.workload_manifest_digest,
        fixture.state_digest,
        fixture.initial_full.selection_digest(),
        fixed_input_sequence_digest(LF_SYNTH_FIXED_INPUT_SEQUENCE),
        fixture.edge_count,
        fixture.state_route_count,
        fixture.state_route_edge_total,
        fixture.vehicle_count,
        fixture.active_count,
        fixture.parked_count,
        typical_edges,
        long_edges,
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        fixture.world.execution_config().worker_count(),
        config.fixed_delta_time_ms(),
        fixture.cost_binding().binding_version(),
        observation_set.observation_tick(),
        observation_set.observation_state_sequence().get(),
        observation_set.digest(),
        cost_model.model_id(),
        cost_model.model_version(),
        fixture.cost_binding().valid_through_tick(),
        fixture.cost_binding().snapshot_sha256(),
        fixture.cost_binding().entry_count(),
        fixture.cost_binding().exact_byte_length(),
        limits.max_entry_count,
        limits.max_exact_byte_length,
        SAMPLE_WARMUP,
        SAMPLE_COUNT,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "steady_tick_before",
        &steady_before,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "observation_full",
        &observation_full,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "observation_delta_zero",
        &observation_delta_zero,
    );
    print_stats("routing-lfsynth-wall-clock", "receiver", &receiver);
    print_stats(
        "routing-lfsynth-wall-clock",
        "candidate_register_1",
        &candidate_one,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "candidate_register_typical",
        &candidate_typical,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "candidate_register_long",
        &candidate_long,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "paired_observation_full",
        &pair_full,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "step_after_observation_full",
        &step_after_full,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "paired_receiver",
        &pair_receiver,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "step_after_receiver",
        &step_after_receiver,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "paired_candidate_register_1",
        &pair_one,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "step_after_candidate_1",
        &step_after_one,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "paired_candidate_register_typical",
        &pair_typical,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "step_after_candidate_typical",
        &step_after_typical,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "paired_candidate_register_long",
        &pair_long,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "step_after_candidate_long",
        &step_after_long,
    );
    print_stats(
        "routing-lfsynth-wall-clock",
        "steady_tick_after",
        &steady_after,
    );
}
