use sha2::{Digest, Sha256};
use std::fs;

use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{
    Artifacts, ResolvedPlan, UrbanCase, Window, compare_runs, run_to_directory,
};

fn artifacts(temp: &tempfile::TempDir) -> Artifacts {
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    let source = temp.path().join("source");
    generate(&config, Scale::Fixture, &source, None).unwrap();
    Artifacts::load(&source).unwrap()
}

#[test]
fn formal_case_expansions_preserve_pr815_frozen_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    let cycle = artifacts
        .catalog()
        .signals
        .iter()
        .map(|signal| signal.cycle_ms)
        .max()
        .unwrap()
        / 16;
    // 9adc198761 的原始运行库、同配置 fixture、完整暖机与两周期观察的逐字节计划摘要。
    // 不用当前实现生成期望值；相同数值统计不能代替全部角色、路线与请求输入一致。
    for (case, digest) in [
        (
            UrbanCase::MixedPeak,
            "5e9dedc9bc14c89c31f7537bd78f0eec1a634af4909662bfbc736ae054ab6f61",
        ),
        (
            UrbanCase::GarageEgress,
            "ad4dbddb43d3e92a6b587c1924c809fbb2a988d424191715221106a155c12683",
        ),
        (
            UrbanCase::GarageIngress,
            "cf39843794fb36b8e056a35e4eae71ab9e8203cd4b56244f84a7c485229c9cfa",
        ),
        (
            UrbanCase::WaitingRelease,
            "b4d3383ad80fecee8522a7e28a4ed7a20caba2351bbd487457261497901b8005",
        ),
        (
            UrbanCase::PermissiveLeft,
            "a78d35ee5927dc440a6e0df0bfd46d07a84ac9839112746b9e2c17299be39bed",
        ),
        (
            UrbanCase::UncontrolledYield,
            "042a8fe3018e58830815133f0cc2496b5324b62d8b0e5fa4fd148980528aec03",
        ),
        (
            UrbanCase::BoundaryBurst,
            "0c3aca5eef8cf89d6b2f84d75882795b18cacc1396b4d8f6e14917da115cb1ad",
        ),
    ] {
        let plan = ResolvedPlan::for_case(
            &artifacts,
            case,
            Window::probe_after(cycle, 2 * cycle).unwrap(),
        )
        .unwrap();
        let bytes = toml::to_string_pretty(&plan).unwrap();
        let observed = Sha256::digest(bytes.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(observed, digest, "{} inputs changed", case.as_str());
    }
}

#[test]
fn sustained_recipe_has_no_finite_demands_or_correctness_roles_and_rejects_leakage() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    let window = Window::probe(256).unwrap();
    let sustained =
        ResolvedPlan::for_case(&artifacts, UrbanCase::SustainedActive, window.clone()).unwrap();
    let mixed = ResolvedPlan::mixed(&artifacts, window).unwrap();
    assert_eq!(sustained.case, "SUSTAINED-ACTIVE");
    assert_eq!(sustained.initial_counts.active, sustained.individuals);
    assert_eq!(
        (
            sustained.initial_counts.parked,
            sustained.initial_counts.completed
        ),
        (0, 0)
    );
    assert!(
        sustained
            .initial
            .iter()
            .all(|vehicle| vehicle.role.is_none() && vehicle.parking.is_none())
    );
    assert!(
        sustained.departures.is_empty()
            && sustained.arrivals.is_empty()
            && sustained.leaves.is_empty()
    );
    assert!(sustained.role_departures.is_empty() && sustained.lifecycle_bursts.is_empty());
    assert!(sustained.boundary_windows.is_empty() && sustained.reservation_rejections.is_empty());
    assert!(sustained.required_per_tile.is_empty());
    assert_eq!((sustained.max_attempts, sustained.retry_ticks), (0, 0));
    assert_eq!(sustained.recycling.as_ref().unwrap().retry_ticks, 8);
    assert!(mixed.recycling.is_none());
    assert_eq!(mixed.arrivals.len(), 2 * mixed.tiles as usize);

    for changed in [
        {
            let mut plan = sustained.clone();
            plan.arrivals = mixed.arrivals.clone();
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.initial[741].role = Some("explicit-arrival".into());
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.recycling = None;
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.recycling.as_mut().unwrap().routes_per_tile[0].clear();
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.max_attempts = 64;
            plan
        },
    ] {
        assert!(changed.validate(&artifacts).is_err());
    }
    let mut duplicate = mixed.clone();
    duplicate.departures.push(duplicate.departures[0].clone());
    assert!(
        duplicate
            .validate(&artifacts)
            .unwrap_err()
            .to_string()
            .contains("重复请求编号")
    );
    let mut overflowing = mixed;
    overflowing.departures[0].sequence = u32::MAX;
    assert!(overflowing.validate(&artifacts).is_err());
    let path = temp.path().join("sustained.toml");
    sustained.write(&path).unwrap();
    let read = ResolvedPlan::read(&path).unwrap();
    assert_eq!(read, sustained);
    read.validate(&artifacts).unwrap();
}

#[test]
fn sustained_replays_across_workers_and_reports_actual_observation_load() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    let plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window::probe_after(32, 768).unwrap(),
    )
    .unwrap();
    let run = |workers, name| {
        let execution =
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::new(workers).unwrap());
        let output = temp.path().join(name);
        let result = run_to_directory(&artifacts, &plan, &output, execution).unwrap();
        assert_eq!(result.status, "probe-complete", "{:?}", result.error);
        assert!(result.replacements > 0);
        assert_eq!(result.exhausted_departures, 0);
        assert_eq!(result.final_counts.1, 0);
        let records: Vec<laneflow_urban_harness::TickRecord> =
            fs::read_to_string(output.join("ticks.jsonl"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        let observed: Vec<_> = records
            .iter()
            .filter(|r| r.tick > plan.window.warm_up_ticks)
            .collect();
        let load = result.active_load.unwrap();
        assert_eq!(load.target, u64::from(plan.initial_counts.active));
        assert_eq!(load.after_step.samples, observed.len());
        assert_eq!(
            load.after_step.min,
            observed.iter().map(|r| r.active as u64).min().unwrap()
        );
        assert_eq!(
            load.before_step.min,
            observed.iter().map(|r| r.intent as u64).min().unwrap()
        );
        assert_eq!(
            load.after_step_below_target_ticks,
            observed
                .iter()
                .filter(|r| r.active < load.target as usize)
                .count()
        );
        assert_eq!(
            load.before_step_below_target_ticks,
            observed
                .iter()
                .filter(|r| r.intent < load.target as usize)
                .count()
        );
        output
    };
    let one = run(1, "one");
    let four = run(4, "four");
    assert_eq!(compare_runs(&one, &four).unwrap().status, "probe-match");
}
