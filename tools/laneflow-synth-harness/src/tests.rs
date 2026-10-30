use std::num::NonZeroU32;

use laneflow_runtime::{ExecutionConfig, TickInput};
use laneflow_static_contract::SignalAspect;
use laneflow_synth_generator::{Case, Identity, Provenance, generate};

use crate::{
    Plan, cycle_ms,
    install::Sample,
    observe::{aspect, motion_valid},
};

fn bundle(case: Case, dt: u32) -> laneflow_synth_generator::Bundle {
    generate(
        Identity::new(10_000, case, dt).unwrap(),
        Provenance {
            source_commit: "a".repeat(40),
            generator_commit: "b".repeat(40),
        },
    )
    .unwrap()
}

#[test]
fn installed_worker_counts_preserve_initial_and_stepped_public_state() {
    let bundle = bundle(Case::W1, 16);
    let mut reference = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    let mut parallel = [2, 4, 8, 16].map(|workers| {
        let execution = ExecutionConfig::new(NonZeroU32::new(workers).unwrap());
        let sample = Sample::new(&bundle, execution).unwrap();
        assert_eq!(sample.world.execution_config(), execution);
        sample
    });
    for tick in 0..=8 {
        if tick != 0 {
            reference.world.step(TickInput::new(16)).unwrap();
            for sample in &mut parallel {
                sample.world.step(TickInput::new(16)).unwrap();
            }
        }
        let expected = reference.observe(tick, true).unwrap();
        let snapshot = reference.world.capture_snapshot().unwrap();
        let digest = laneflow_runtime::deterministic_state_digest(&snapshot).unwrap();
        for sample in &mut parallel {
            assert_eq!(sample.observe(tick, true).unwrap(), expected);
            assert_eq!(
                sample.world.latest_transition_events(),
                reference.world.latest_transition_events()
            );
            let snapshot = sample.world.capture_snapshot().unwrap();
            assert_eq!(
                laneflow_runtime::deterministic_state_digest(&snapshot).unwrap(),
                digest
            );
        }
    }
    assert!(Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::new(17).unwrap())).is_err());
}

#[test]
fn frozen_signal_boundaries_and_offsets() {
    assert_eq!(cycle_ms(16), 58_032);
    assert_eq!(cycle_ms(33), 58_113);
    for (dt, ticks) in [
        (16, [1_875, 188, 63, 1_250, 188, 63]),
        (33, [910, 91, 31, 607, 91, 31]),
    ] {
        let expected = [
            [SignalAspect::Green, SignalAspect::Red],
            [SignalAspect::Yellow, SignalAspect::Red],
            [SignalAspect::Red, SignalAspect::Red],
            [SignalAspect::Red, SignalAspect::Green],
            [SignalAspect::Red, SignalAspect::Yellow],
            [SignalAspect::Red, SignalAspect::Red],
        ];
        let mut tick = 0;
        for (count, states) in ticks.into_iter().zip(expected) {
            for (route, expected) in states.into_iter().enumerate() {
                assert_eq!(aspect(0, route as u32, tick, dt), expected);
                assert_eq!(aspect(0, route as u32, tick + count - 1, dt), expected);
            }
            tick += count;
        }
        assert_eq!(aspect(0, 0, tick, dt), SignalAspect::Green);
        assert_eq!(
            aspect(
                1,
                0,
                (30_000_u64.div_ceil(u64::from(dt)) * u64::from(dt) - 997).div_ceil(u64::from(dt)),
                dt
            ),
            SignalAspect::Yellow
        );
    }
}

#[test]
fn motion_guard_rejects_backwards_overspeed_and_impossible_acceleration() {
    assert!(motion_valid(1_000, 1_176, 10, 11, 16));
    assert!(!motion_valid(1_000, 999, 10, 10, 16));
    assert!(!motion_valid(0, 222_402, 13_900, 13_900, 16));
    assert!(!motion_valid(0, 0, 13_900, 13_901, 16));
    assert!(!motion_valid(0, 0, 0, 100, 16));
    // #1159：整数硬停止投影清速度；emergency_decel 不是已提交速度差的绝对上限。
    assert!(motion_valid(0, 0, 1_000, 0, 16));
}

#[test]
fn canonical_initialization_and_guard_negative_controls() {
    let bundle = bundle(Case::W1, 16);
    let plan = Plan::read(&bundle).unwrap();
    assert_eq!(
        (plan.warm_up_ticks, plan.observation_ticks),
        (14_508, 29_016)
    );
    let mut sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    assert_eq!(
        sample.observe(0, true).unwrap().to_hex().as_str(),
        "ebb35b54634106fdc227b7b3f86712232ad4e30c75512e556aa538771cf73a7e"
    );
    for tick in 1..=8 {
        let outcome = sample.world.step(TickInput::new(16)).unwrap();
        sample
            .observe(tick, outcome.parking_arrivals().is_empty())
            .unwrap();
    }
    assert!(sample.observe(8, false).is_err());
    assert!(sample.observe(9, true).is_err());
    let cursor = sample.cursor;
    sample.cursor += 1;
    assert!(sample.observe(8, true).is_err());
    sample.cursor = cursor;
    sample.groups.swap(0, 1);
    assert!(sample.observe(8, true).is_err());
    sample.groups.swap(0, 1);
    sample.handles.swap(0, 1);
    assert!(sample.observe(8, true).is_err());
    sample.handles.swap(0, 1);
    sample.bindings[62] = None;
    assert!(sample.observe(8, true).is_err());
}

#[test]
fn invalid_input_and_existing_output_are_preserved() {
    let bundle = bundle(Case::W3, 33);
    assert!(Plan::read(&bundle).is_ok());
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let output = root.path().join("output");
    bundle.write_new(&input).unwrap();
    std::fs::write(input.join("input-plan.json"), b"{}").unwrap();
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    let before = std::fs::read(output.join("report.json")).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(report["complete"], false);
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    assert_eq!(std::fs::read(output.join("report.json")).unwrap(), before);
}

#[test]
fn w2_is_fully_active_and_shortened_plan_is_rejected() {
    let bundle = bundle(Case::W2, 33);
    let plan = Plan::read(&bundle).unwrap();
    assert_eq!(
        (plan.warm_up_ticks, plan.observation_ticks),
        (7_044, 14_088)
    );
    let mut sample = Sample::new(&bundle, ExecutionConfig::new(NonZeroU32::MIN)).unwrap();
    sample.observe(0, true).unwrap();
    assert!(
        sample
            .initial
            .iter()
            .all(|state| state.status() == laneflow_runtime::VehicleStatus::Active)
    );
    for tick in 1..=8 {
        sample.world.step(TickInput::new(33)).unwrap();
        sample.observe(tick, true).unwrap();
    }
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    bundle.write_new(&input).unwrap();
    let mut plan: serde_json::Value =
        serde_json::from_slice(&bundle.files()["input-plan.json"]).unwrap();
    plan["observation_ticks"] = 1.into();
    let mut bytes = serde_json::to_vec(&plan).unwrap();
    bytes.push(b'\n');
    std::fs::write(input.join("input-plan.json"), bytes).unwrap();
    let output = root.path().join("output");
    assert!(
        crate::run(
            &input,
            &"c".repeat(40),
            ExecutionConfig::new(NonZeroU32::MIN),
            &output
        )
        .is_err()
    );
    assert!(!output.join("ticks.bin").exists());
}
