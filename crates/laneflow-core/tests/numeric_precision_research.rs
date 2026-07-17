use half::f16;
use laneflow_core::{CoreEvent, CoreWorld, TickInput, VehicleStatus};

#[allow(dead_code)]
#[path = "support/numeric_precision_research.rs"]
mod candidates;
#[allow(dead_code)]
#[path = "support/vehicle_following_scenarios.rs"]
mod scenarios;

use candidates::{
    CandidateLayout, CandidateScenario, CandidateStatus, CandidateWorld, CompensatedF32Mode,
    F64Mode, MixedF32Mode, PrecisionMode, RawF32Mode, STEP_COUNT, VEHICLE_COUNT, constant_addition,
    finite_candidate_value,
};

const F64_MODEL_EPSILON: f64 = 1.0e-8;

fn core_world(
    vehicle_count: usize,
    scenario: CandidateScenario,
    layout: CandidateLayout,
) -> CoreWorld {
    match (scenario, layout) {
        (CandidateScenario::FreeFlow, CandidateLayout::LegacySingleEdge) => {
            scenarios::free_flow_world(vehicle_count)
        }
        (CandidateScenario::DensePlatoon, CandidateLayout::LegacySingleEdge) => {
            scenarios::dense_platoon_world(vehicle_count)
        }
        (CandidateScenario::StopAndGo, CandidateLayout::LegacySingleEdge) => {
            scenarios::stop_and_go_world(vehicle_count)
        }
        (CandidateScenario::FreeFlow, CandidateLayout::LocalityPreserving) => {
            scenarios::locality_free_flow_world(vehicle_count)
        }
        (CandidateScenario::DensePlatoon, CandidateLayout::LocalityPreserving) => {
            scenarios::locality_dense_platoon_world(vehicle_count)
        }
        (CandidateScenario::StopAndGo, CandidateLayout::LocalityPreserving) => {
            scenarios::locality_stop_and_go_world(vehicle_count)
        }
    }
}

fn step_core(world: &mut CoreWorld) -> (Vec<(usize, usize)>, usize) {
    let result = world
        .step(TickInput::new(scenarios::FIXED_DELTA_TIME_MS))
        .expect("reference Core step must succeed");
    let handles = world
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();
    let index_of = |handle| {
        handles
            .iter()
            .position(|candidate| *candidate == handle)
            .expect("event handle must identify a live vehicle")
    };
    let mut projections = Vec::new();
    let mut edge_changes = 0;
    for event in result.events {
        match event {
            CoreEvent::VehicleFollowingSafetyProjectionApplied(event) => {
                projections.push((index_of(event.vehicle), index_of(event.leader)));
            }
            CoreEvent::VehicleChangedEdge(_) => edge_changes += 1,
            unexpected => panic!("unexpected reference event: {unexpected:?}"),
        }
    }
    (projections, edge_changes)
}

fn assert_f64_model_matches_core(
    vehicle_count: usize,
    scenario: CandidateScenario,
    layout: CandidateLayout,
) {
    let mut reference = core_world(vehicle_count, scenario, layout);
    let mut candidate = CandidateWorld::<F64Mode>::new(vehicle_count, scenario, layout);
    for _ in 0..STEP_COUNT {
        let (reference_projections, reference_edge_changes) = step_core(&mut reference);
        let summary = candidate.step();
        assert_eq!(summary.safety_projections, reference_projections);
        assert_eq!(summary.edge_changes, reference_edge_changes);
        for (index, reference_state) in reference.vehicles().enumerate() {
            let candidate_state = candidate.snapshot(index);
            assert_eq!(
                candidate_state.route_edge_index,
                reference_state.route_edge_index
            );
            assert_eq!(
                candidate_state.status,
                match reference_state.status {
                    VehicleStatus::Active => CandidateStatus::Active,
                    VehicleStatus::Stopped => CandidateStatus::Stopped,
                    unexpected => panic!("unexpected reference status: {unexpected:?}"),
                }
            );
            assert!(
                (candidate_state.edge_progress - reference_state.edge_progress.value()).abs()
                    <= F64_MODEL_EPSILON,
                "scenario={scenario:?} layout={layout:?} index={index} candidate={candidate_state:?} reference={reference_state:?}"
            );
            assert!(
                (candidate_state.current_speed - reference_state.current_speed.value()).abs()
                    <= F64_MODEL_EPSILON
            );
            assert!(
                (candidate_state.applied_acceleration
                    - reference_state.applied_acceleration.value())
                .abs()
                    <= F64_MODEL_EPSILON
            );
        }
    }
}

#[test]
fn f64_research_model_matches_core_control_flow() {
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            assert_f64_model_matches_core(256, scenario, layout);
        }
    }
}

#[test]
#[ignore = "10k f64 candidate-oracle alignment is an explicit #122 research measurement"]
fn f64_research_model_matches_core_at_10k() {
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            assert_f64_model_matches_core(VEHICLE_COUNT, scenario, layout);
        }
    }
}

#[test]
fn compensated_and_mixed_progress_reduce_repeated_addition_drift() {
    let expected = 1_000.0;
    let raw = constant_addition::<RawF32Mode>(0.0, 0.16, 6_250);
    let compensated = constant_addition::<CompensatedF32Mode>(0.0, 0.16, 6_250);
    let mixed = constant_addition::<MixedF32Mode>(0.0, 0.16, 6_250);
    assert!((raw - expected).abs() > 0.01);
    assert!((compensated - expected).abs() <= 0.01);
    assert!((mixed - expected).abs() <= 0.01);
}

fn assert_candidate_replay_is_exact<M: PrecisionMode>() {
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            let mut first = CandidateWorld::<M>::new(256, scenario, layout);
            let mut second = CandidateWorld::<M>::new(256, scenario, layout);
            for tick in 1..=120 {
                assert_eq!(
                    first.step(),
                    second.step(),
                    "mode={} layout={layout:?} scenario={scenario:?} tick={tick}",
                    M::NAME,
                );
                for index in 0..first.len() {
                    assert_eq!(
                        first.snapshot(index),
                        second.snapshot(index),
                        "mode={} layout={layout:?} scenario={scenario:?} tick={tick} vehicle={index}",
                        M::NAME,
                    );
                }
            }
        }
    }
}

#[test]
fn numeric_candidates_replay_deterministically_on_the_same_runtime() {
    assert_candidate_replay_is_exact::<F64Mode>();
    assert_candidate_replay_is_exact::<RawF32Mode>();
    assert_candidate_replay_is_exact::<CompensatedF32Mode>();
    assert_candidate_replay_is_exact::<MixedF32Mode>();
}

fn print_long_duration_addition<M: PrecisionMode>(speed: f64, ticks: usize) {
    let travel_per_tick = speed * scenarios::FIXED_DELTA_TIME_MS as f64 / 1_000.0;
    let expected = travel_per_tick * ticks as f64;
    let actual = constant_addition::<M>(0.0, travel_per_tick, ticks);
    eprintln!(
        "numeric_long_addition mode={} speed={} ticks={} expected={} actual={} absolute_error={}",
        M::NAME,
        speed,
        ticks,
        expected,
        actual,
        (actual - expected).abs(),
    );
}

#[test]
#[ignore = "long-duration repeated-addition matrix is an explicit #122 research measurement"]
fn numeric_candidate_long_duration_addition_report() {
    for speed in [1.0, 10.0, 30.0] {
        print_long_duration_addition::<F64Mode>(speed, 36_000);
        print_long_duration_addition::<RawF32Mode>(speed, 36_000);
        print_long_duration_addition::<CompensatedF32Mode>(speed, 36_000);
        print_long_duration_addition::<MixedF32Mode>(speed, 36_000);
    }
}

#[test]
fn raw_f16_exceeds_runtime_and_heading_error_ceilings() {
    let cases = [
        (16_384.0_f32, 0.01_f32),
        (100.0, 0.01),
        (128.0, 0.01),
        (std::f32::consts::PI, 0.000_1),
    ];
    for (value, ceiling) in cases {
        let quantized = f16::from_f32(value).to_f32();
        let next = f16::from_bits(f16::from_f32(value).to_bits() + 1).to_f32();
        let representation_floor = (next - quantized).abs() * 0.5;
        assert!(representation_floor > ceiling);
    }
}

#[test]
fn checked_f32_boundary_conversion_rejects_overflow_and_canonicalizes_zero() {
    assert_eq!(finite_candidate_value::<f32>(-0.0).unwrap().to_bits(), 0);
    assert!(finite_candidate_value::<f32>(f64::NAN).is_none());
    assert!(finite_candidate_value::<f32>(f64::INFINITY).is_none());
    assert!(finite_candidate_value::<f32>(f64::MAX).is_none());

    let edge_length = 16_384.0_f64;
    assert_eq!((edge_length - 1.0e-9) as f32, edge_length as f32);
    let lower_ulp = edge_length as f32 - (edge_length as f32).next_down();
    let upper_ulp = (edge_length as f32).next_up() - edge_length as f32;
    assert_eq!(lower_ulp, 0.000_976_562_5);
    assert_eq!(upper_ulp, 0.001_953_125);
    assert!(4.0 * f64::from(upper_ulp) < 0.01);
}

#[derive(Clone, Debug, Default)]
struct DifferentialStats {
    max_progress_error: f64,
    max_progress_relative_error: f64,
    max_speed_error: f64,
    max_speed_relative_error: f64,
    max_acceleration_error: f64,
    max_acceleration_relative_error: f64,
    first_discrete_divergence: Option<String>,
}

impl DifferentialStats {
    fn observe<M: PrecisionMode>(
        &mut self,
        tick: usize,
        reference_summary: &candidates::CandidateStepSummary,
        candidate_summary: &candidates::CandidateStepSummary,
        reference: &CandidateWorld<F64Mode>,
        candidate: &CandidateWorld<M>,
    ) {
        if self.first_discrete_divergence.is_none() && reference_summary != candidate_summary {
            self.first_discrete_divergence = Some(format!(
                "tick={tick} reference_events={reference_summary:?} candidate_events={candidate_summary:?}"
            ));
        }
        for index in 0..reference.len() {
            let reference = reference.snapshot(index);
            let candidate = candidate.snapshot(index);
            if self.first_discrete_divergence.is_none()
                && (reference.route_edge_index != candidate.route_edge_index
                    || reference.status != candidate.status)
            {
                self.first_discrete_divergence = Some(format!(
                    "tick={tick} vehicle={index} reference={reference:?} candidate={candidate:?}"
                ));
            }
            let progress_error = (reference.route_progress - candidate.route_progress).abs();
            let speed_error = (reference.current_speed - candidate.current_speed).abs();
            let acceleration_error =
                (reference.applied_acceleration - candidate.applied_acceleration).abs();
            self.max_progress_error = self.max_progress_error.max(progress_error);
            self.max_speed_error = self.max_speed_error.max(speed_error);
            self.max_acceleration_error = self.max_acceleration_error.max(acceleration_error);
            if reference.route_progress.abs() >= 1.0 {
                self.max_progress_relative_error = self
                    .max_progress_relative_error
                    .max(progress_error / reference.route_progress.abs());
            }
            if reference.current_speed.abs() >= 1.0 {
                self.max_speed_relative_error = self
                    .max_speed_relative_error
                    .max(speed_error / reference.current_speed.abs());
            }
            if reference.applied_acceleration.abs() >= 1.0 {
                self.max_acceleration_relative_error = self
                    .max_acceleration_relative_error
                    .max(acceleration_error / reference.applied_acceleration.abs());
            }
        }
    }

    fn accepted(&self) -> bool {
        self.first_discrete_divergence.is_none()
            && self.max_progress_error <= 0.01
            && self.max_progress_relative_error <= 1.0e-5
            && self.max_speed_error <= 0.01
            && self.max_speed_relative_error <= 1.0e-4
            && self.max_acceleration_error <= 0.02
            && self.max_acceleration_relative_error <= 1.0e-3
    }
}

fn run_differential_matrix(
    vehicle_count: usize,
    scenario: CandidateScenario,
    layout: CandidateLayout,
) -> [DifferentialStats; 3] {
    let mut reference = CandidateWorld::<F64Mode>::new(vehicle_count, scenario, layout);
    let mut raw = CandidateWorld::<RawF32Mode>::new(vehicle_count, scenario, layout);
    let mut compensated =
        CandidateWorld::<CompensatedF32Mode>::new(vehicle_count, scenario, layout);
    let mut mixed = CandidateWorld::<MixedF32Mode>::new(vehicle_count, scenario, layout);
    let mut stats = std::array::from_fn(|_| DifferentialStats::default());
    for tick in 1..=STEP_COUNT {
        let reference_summary = reference.step();
        let raw_summary = raw.step();
        let compensated_summary = compensated.step();
        let mixed_summary = mixed.step();
        stats[0].observe(tick, &reference_summary, &raw_summary, &reference, &raw);
        stats[1].observe(
            tick,
            &reference_summary,
            &compensated_summary,
            &reference,
            &compensated,
        );
        stats[2].observe(tick, &reference_summary, &mixed_summary, &reference, &mixed);
    }
    stats
}

#[test]
#[ignore = "10k f32/mixed differential matrix is an explicit #122 research measurement"]
fn numeric_candidate_differential_report_10k() {
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        for scenario in [
            CandidateScenario::FreeFlow,
            CandidateScenario::DensePlatoon,
            CandidateScenario::StopAndGo,
        ] {
            let stats = run_differential_matrix(VEHICLE_COUNT, scenario, layout);
            for (mode, stats) in [
                (RawF32Mode::NAME, &stats[0]),
                (CompensatedF32Mode::NAME, &stats[1]),
                (MixedF32Mode::NAME, &stats[2]),
            ] {
                eprintln!(
                    "numeric_candidate_diff layout={} scenario={} mode={} accepted={} max_progress_error={:.12} max_progress_relative_error={:.12} max_speed_error={:.12} max_speed_relative_error={:.12} max_acceleration_error={:.12} max_acceleration_relative_error={:.12} first_discrete_divergence={:?}",
                    layout.benchmark_name(),
                    scenario.benchmark_name(),
                    mode,
                    stats.accepted(),
                    stats.max_progress_error,
                    stats.max_progress_relative_error,
                    stats.max_speed_error,
                    stats.max_speed_relative_error,
                    stats.max_acceleration_error,
                    stats.max_acceleration_relative_error,
                    stats.first_discrete_divergence,
                );
            }
        }
    }
}

#[test]
#[ignore = "100k dense f32/mixed differential observation is an explicit #122 research measurement"]
fn numeric_candidate_differential_report_100k_dense_observation() {
    for layout in [
        CandidateLayout::LegacySingleEdge,
        CandidateLayout::LocalityPreserving,
    ] {
        let stats = run_differential_matrix(
            candidates::SCALING_VEHICLE_COUNT,
            CandidateScenario::DensePlatoon,
            layout,
        );
        for (mode, stats) in [
            (RawF32Mode::NAME, &stats[0]),
            (CompensatedF32Mode::NAME, &stats[1]),
            (MixedF32Mode::NAME, &stats[2]),
        ] {
            eprintln!(
                "numeric_candidate_diff_100k layout={} scenario=dense_platoon mode={} accepted={} max_progress_error={:.12} max_progress_relative_error={:.12} max_speed_error={:.12} max_speed_relative_error={:.12} max_acceleration_error={:.12} max_acceleration_relative_error={:.12} first_discrete_divergence={:?}",
                layout.benchmark_name(),
                mode,
                stats.accepted(),
                stats.max_progress_error,
                stats.max_progress_relative_error,
                stats.max_speed_error,
                stats.max_speed_relative_error,
                stats.max_acceleration_error,
                stats.max_acceleration_relative_error,
                stats.first_discrete_divergence,
            );
        }
    }
}

fn print_memory<M: PrecisionMode>(vehicle_count: usize) {
    let world = CandidateWorld::<M>::new(
        vehicle_count,
        CandidateScenario::DensePlatoon,
        CandidateLayout::LocalityPreserving,
    );
    let stats = world.memory_stats();
    eprintln!(
        "numeric_candidate_memory mode={} vehicles={} vehicle_size={} motion_size={} retained_bytes={} bytes_per_vehicle={:.2}",
        M::NAME,
        vehicle_count,
        stats.vehicle_size,
        stats.motion_size,
        stats.retained_bytes,
        stats.retained_bytes as f64 / vehicle_count as f64,
    );
}

#[test]
#[ignore = "candidate layout memory is an explicit #122 research measurement"]
fn numeric_candidate_memory_report() {
    for vehicle_count in [VEHICLE_COUNT, candidates::SCALING_VEHICLE_COUNT] {
        print_memory::<F64Mode>(vehicle_count);
        print_memory::<RawF32Mode>(vehicle_count);
        print_memory::<CompensatedF32Mode>(vehicle_count);
        print_memory::<MixedF32Mode>(vehicle_count);
    }
}

fn max_f16_roundtrip_error(minimum: f32, maximum: f32) -> f32 {
    let mut values = (0..=u16::MAX)
        .map(|bits| f16::from_bits(bits).to_f32())
        .filter(|value| value.is_finite() && *value >= minimum && *value <= maximum)
        .collect::<Vec<_>>();
    values.sort_unstable_by(f32::total_cmp);
    values.dedup();
    values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) * 0.5)
        .fold(0.0_f32, f32::max)
}

#[test]
#[ignore = "f16 and fixed quantization matrix is an explicit #122 research measurement"]
fn numeric_quantization_matrix_report() {
    let domains = [
        ("progress", 0.0, 16_384.0, 0.01),
        ("speed", 0.0, 100.0, 0.01),
        ("acceleration", -50.0, 50.0, 0.02),
        ("extent_offset", -128.0, 128.0, 0.01),
        (
            "heading",
            -std::f32::consts::PI,
            std::f32::consts::PI,
            0.000_1,
        ),
    ];
    for (domain, minimum, maximum, ceiling) in domains {
        let error = max_f16_roundtrip_error(minimum, maximum);
        eprintln!(
            "numeric_f16_quantization domain={} minimum={} maximum={} max_roundtrip_error={} ceiling={} accepted={}",
            domain,
            minimum,
            maximum,
            error,
            ceiling,
            error <= ceiling,
        );
    }
    let heading_step = std::f64::consts::TAU / 65_536.0;
    eprintln!(
        "numeric_integer_quantization domain=progress format=u32_centimeter range_max={} max_error={} accepted={}",
        u32::MAX as f64 / 100.0,
        0.005,
        true,
    );
    eprintln!(
        "numeric_integer_quantization domain=speed format=u16_centimeter_per_second range_max={} max_error={} accepted={}",
        u16::MAX as f64 / 100.0,
        0.005,
        true,
    );
    eprintln!(
        "numeric_integer_quantization domain=acceleration format=i16_centimeter_per_second_squared range_min={} range_max={} max_error={} accepted={}",
        i16::MIN as f64 / 100.0,
        i16::MAX as f64 / 100.0,
        0.005,
        true,
    );
    eprintln!(
        "numeric_integer_quantization domain=heading format=u16_turn range=[-pi,pi) step={} max_error={} accepted={}",
        heading_step,
        heading_step * 0.5,
        heading_step * 0.5 <= 0.000_1,
    );
}
