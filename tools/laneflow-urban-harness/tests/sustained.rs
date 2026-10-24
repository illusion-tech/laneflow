use std::fs;

use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{
    Artifacts, Diagnostics, ResolvedPlan, UrbanCase, Window, compare_runs, run_to_directory,
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
    // #1042 改为文件字节摘要（BLAKE3-256）：同一步骤先确认当前计划字节的 SHA-256 仍等于
    // 原金样本（与历史字节逐字节相同），再记录同一字节的新摘要，核对记录见 PR。
    for (case, digest) in [
        (
            UrbanCase::MixedPeak,
            "067077e81a84a8cc8d42cfac2cc4276f3f6d247c033808f67860a187459461ae",
        ),
        (
            UrbanCase::GarageEgress,
            "724f8efaf710b06916bd6fe31f5f10926c929437d03d439b3bc7b6542426443f",
        ),
        (
            UrbanCase::GarageIngress,
            "359093d41f20f7a12acf26a72dabac7b18b8744d4f6f6b50876b627443f71286",
        ),
        (
            UrbanCase::WaitingRelease,
            "a8db2daa65751daad830ecdc9182cfbcd2c18f8486c560b9606d7c5133eec6e9",
        ),
        (
            UrbanCase::PermissiveLeft,
            "d75a7990a0d5b586a3c5a2bbc7f6c2a18d72afe7a0137c17d252ebf158cd5590",
        ),
        // 唯一有意偏离 9adc198761 的一项：那时冻结的让行角色走 c04.n0-e0，与主路脉冲
        // c04.w1-e1 没有共同冲突区，真实让行关系从未出现；改走同出口车道汇入的 c04.n0-e1。
        (
            UrbanCase::UncontrolledYield,
            "654007b09b63e72149a093fe0ec6d96ecb0fb3991fc5825e0f38aab779685d00",
        ),
        (
            UrbanCase::BoundaryBurst,
            "9f210ec94ad896686bb9b7a8a70f9669e5016eebb4455ebcdc5b58cfa431a44f",
        ),
    ] {
        let mut plan = ResolvedPlan::for_case(
            &artifacts,
            case,
            Window::probe_after(cycle, 2 * cycle).unwrap(),
        )
        .unwrap();
        plan.validate(&artifacts).unwrap();
        // #819 只校准 compiler_controlled_peak_bytes，但完整 manifest 摘要随之改变。
        // 保持 PR #815 的旧计划摘要不动：比较角色/路线/请求时仅恢复当时的报告绑定。
        // 生产校验仍要求新计划绑定当前 manifest；归一化后的比较副本不可用于执行。
        let current_manifest_digest = plan.manifest_digest.clone();
        plan.manifest_digest =
            "4ec17200ec09c682124ec08d85e59e3fcaeccbd4611899998021fdbf0211b3ee".into();
        if current_manifest_digest != plan.manifest_digest {
            assert!(plan.validate(&artifacts).is_err());
        }
        let bytes = toml::to_string_pretty(&plan).unwrap();
        let mut preimage = b"laneflow:urban-harness-file:v1\0".to_vec();
        preimage.extend_from_slice(bytes.as_bytes());
        let observed = blake3::hash(&preimage).to_hex();
        assert_eq!(
            observed.as_str(),
            digest,
            "{} inputs changed",
            case.as_str()
        );
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
            plan.recycling.as_mut().unwrap().candidate_order = "unknown".into();
            plan
        },
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
            plan.recycling.as_mut().unwrap().entries_per_tile[0].clear();
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.recycling.as_mut().unwrap().attempts_per_boundary = 0;
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.recycling.as_mut().unwrap().progress_mm[0] = 0;
            plan
        },
        {
            let mut plan = sustained.clone();
            plan.recycling.as_mut().unwrap().entries_per_tile[0][0]
                .routes
                .clear();
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
        let result =
            run_to_directory(&artifacts, &plan, &output, execution, Diagnostics::Enabled).unwrap();
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
