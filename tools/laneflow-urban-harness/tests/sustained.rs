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

// PR #815 的编制配置中尚无车型构型及刚体几何，只去掉各车型表中的这十个字段。
fn pr815_config(config: &str) -> String {
    let mut in_profile = false;
    config
        .split_inclusive('\n')
        .filter(|line| {
            if line.starts_with('[') {
                in_profile = line.trim_end() == "[[profiles]]";
            }
            !in_profile
                || !line.split_once(" = ").is_some_and(|(key, _)| {
                    matches!(
                        key,
                        "configuration"
                            | "wheelbase_m"
                            | "track_width_m"
                            | "reference_height_m"
                            | "rear_overhang_m"
                            | "width_m"
                            | "bottom_height_m"
                            | "top_height_m"
                            | "max_steering_angle_rad"
                            | "max_steering_rate_rad_s"
                    )
                })
        })
        .collect()
}

#[test]
fn formal_case_expansions_preserve_pr815_frozen_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = artifacts(&temp);
    // 路线目录只替换文件头的修订身份，随后逐字节核对 PR #815 的所有路线。
    use sha2::{Digest, Sha256};
    let routes = fs::read_to_string(temp.path().join("source/routes.toml")).unwrap();
    let sha256 = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()
    };
    let current = format!(
        "network_revision = \"{}\"",
        artifacts.catalog().network_revision
    );
    assert_eq!(routes.matches(&current).count(), 1);
    let historical = routes.replacen(
        &current,
        "network_revision = \"1d427488a89af7f0b2601b200a867a413529cdc358a7f7e2bfbabe4a27bd8819\"",
        1,
    );
    let historical_routes = "e9e6a6b260d633bcbb04923cb3922c673dfc214cd77bd6e32d4a1ec279ef70de";
    assert_eq!(sha256(historical.as_bytes()), historical_routes);
    // 长度和 SHA-256 均来自 9adc198761 的 fixture/manifest.toml。
    let config = pr815_config(&fs::read_to_string(temp.path().join("source/config.toml")).unwrap());
    let historical_config = "b04b9153596cc9bf28b38e341fd96c6cdaab2596384f8fe3d49e4a5308aa037d";
    assert_eq!(config.len(), 1_300);
    assert_eq!(sha256(config.as_bytes()), historical_config);
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
        // 生产路径已按当前制品校验。比较 PR #815 的角色、路线和请求时，在副本中恢复
        // 当时的清单绑定及各文件的历史摘要和长度。上面已核对路线目录和配置的历史字节，
        // 生成器测试核对当前制品的完整摘要。
        plan.manifest_digest =
            "4ec17200ec09c682124ec08d85e59e3fcaeccbd4611899998021fdbf0211b3ee".into();
        for (name, pr815_sha256) in [
            ("routes.toml", historical_routes),
            ("config.toml", historical_config),
            (
                "common.lfre",
                "82b8167c2a8bfb3b06f940d8e73c975d0935fb0841b9373387e8f9a422456109",
            ),
            (
                "topology.lfre",
                "e64fe0958b6d558abc04af993e7ffd98e5ed97012c936370dcc8bb9e8873f2eb",
            ),
            (
                "network.lfca",
                "910d0ce5598f213d652f37b786615530981b99fcc482f9712ce49afad12edb4c",
            ),
        ] {
            plan.files.get_mut(name).unwrap().sha256 = pr815_sha256.into();
        }
        // LFRE 8 增加车型构型及几何。历史计划绑定的是原始配置与公共模块长度。
        plan.files.get_mut("config.toml").unwrap().bytes = 1_300;
        plan.files.get_mut("common.lfre").unwrap().bytes = 1_024;
        // LFRE 6 把每条编制车道的边引用写成向量；PR #815 的固定计划绑定了当时的来源长度。
        plan.files.get_mut("topology.lfre").unwrap().bytes = 1_669_184;
        // LFCA 6 增加第九节，历史输入对照使用旧版文件长度。
        plan.files.get_mut("network.lfca").unwrap().bytes = 8_868_299;
        assert!(plan.validate(&artifacts).is_err());
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
