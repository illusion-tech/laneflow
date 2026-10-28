//! #529：editable load 停顿的同机描述性取证（快照合同 §8），只进入测试二进制。
//!
//! 准备：生成城市 `LFRE` 来源与制品，以发布来源运行 harness 世界若干拍，经同修订换根
//! 切到编辑来源后保存 `LFRS`。随后按样本分段计时宿主的 editable load：重编译（逐模块
//! 准入、编译、发射、后发射检查）、修订构建、编辑来源修订认证与 `restore_lfrs`。数值只
//! 描述本机，不作产品阈值，不与发布来源初值作回归比较。
//!
//! ```text
//! cargo +1.99.1 test --release --locked -p laneflow-urban-harness --lib editable_load_evidence -- --ignored --nocapture
//! ```
//!
//! `LF_EDITABLE_LOAD_SCALE`（`fixture` / `10k` / `100k`，缺省 `fixture`）选择规模，
//! `LF_EDITABLE_LOAD_SAMPLES`（缺省 9）选择样本数。

use std::{fs, num::NonZeroU32, time::Instant};

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, PortableDiffBase, PortableEmissionProvenance,
    emit_portable_candidate, road_editing::RoadEditingModuleInput,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverPreflightLimits, EditableDiffBase, ExecutionConfig,
    LfcaOriginBinding, MigrationPolicyKind, NetworkRevisionCutoverDescriptor, RoadEditingState,
    RoadEditingStateModule, SnapshotRestoreLimits, attest_editable_shared_network_revision,
    encode_lfrs, restore_lfrs,
};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, RecompileFailure, SharedNetworkBuildLimits,
    SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_urban_generator::{
    BUILD_ID, COMMON_DOCUMENT_KEY, DOCUMENT_KEY, Scale, UrbanConfig, generate,
};

use crate::{Artifacts, Harness, ResolvedPlan, Window, host::Host};

const WARM_TICKS: u64 = 64;
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648);
const RESTORE_LIMITS: SnapshotRestoreLimits = SnapshotRestoreLimits::new(2_147_483_648, 4_096);

fn build_options() -> SharedNetworkBuildOptions<'static> {
    // 与 `Artifacts::load` 的 headless 宿主同一组参数。
    SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS)
}

/// 宿主重编译：与生成器同一编译限额、发射 provenance 与创世差异基线。
fn recompile(state: &RoadEditingState) -> Result<Vec<u8>, RecompileFailure> {
    let content = |error: &dyn std::fmt::Debug| RecompileFailure::Content(format!("{error:?}"));
    let mut unit = CompilationUnitBuilder::new(CompileLimits::single_network_1m_v2());
    for module in state.modules() {
        let input = RoadEditingModuleInput::try_new(
            module.source_document_key(),
            module.source_bytes(),
            None,
        )
        .map_err(|error| content(&error))?;
        unit.add_road_editing_module(input)
            .map_err(|error| content(&error))?;
    }
    let output = Compiler::new()
        .compile(unit.build().map_err(|error| content(&error))?)
        .map_err(|bundle| content(&bundle.diagnostics()))?;
    let provenance = PortableEmissionProvenance::try_new(BUILD_ID).map_err(|e| content(&e))?;
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|error| content(&error))?;
    drop(output);
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(|error| content(&error))?;
    Ok(candidate.canonical_artifact().bytes().to_vec())
}

fn build(lfca: &[u8]) -> std::sync::Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(lfca, FormatLimits::HARD).expect("checked LFCA");
    build_shared_network_revision(input, build_options()).expect("shared network revision")
}

fn median_ms(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn env_scale() -> Scale {
    match std::env::var("LF_EDITABLE_LOAD_SCALE") {
        Ok(value) => Scale::parse(&value).expect("LF_EDITABLE_LOAD_SCALE"),
        Err(std::env::VarError::NotPresent) => Scale::Fixture,
        Err(error) => panic!("invalid LF_EDITABLE_LOAD_SCALE: {error}"),
    }
}

fn env_samples() -> usize {
    match std::env::var("LF_EDITABLE_LOAD_SAMPLES") {
        Ok(value) => value.parse().expect("LF_EDITABLE_LOAD_SAMPLES"),
        Err(std::env::VarError::NotPresent) => 9,
        Err(error) => panic!("invalid LF_EDITABLE_LOAD_SAMPLES: {error}"),
    }
}

#[test]
#[ignore = "manual same-machine evidence for #529; see module docs"]
fn editable_load_evidence() {
    let scale = env_scale();
    let samples = env_samples();
    assert!(samples > 0, "at least one sample");
    let execution = ExecutionConfig::new(NonZeroU32::MIN);
    let temp = tempfile::tempdir().expect("temporary directory");
    let source = temp.path().join("source");
    let config = UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml"))
        .expect("urban configuration");
    generate(&config, scale, &source, None).expect("generate urban artifacts");
    let artifacts = Artifacts::load(&source).expect("load artifacts");

    // 编辑来源：宿主持有的两份 LFRE 模块。
    let common = fs::read(source.join("common.lfre")).expect("common.lfre");
    let topology = fs::read(source.join("topology.lfre")).expect("topology.lfre");
    let lfre_bytes = common.len() + topology.len();
    let state = RoadEditingState::try_new(vec![
        RoadEditingStateModule::new(COMMON_DOCUMENT_KEY.to_owned(), common),
        RoadEditingStateModule::new(DOCUMENT_KEY.to_owned(), topology),
    ])
    .expect("road editing state");
    let published_lfca = fs::read(source.join("network.lfca")).expect("network.lfca");
    let recompiled = recompile(&state).expect("host recompile");
    assert_eq!(
        recompiled, published_lfca,
        "host recompile must reproduce the generator LFCA byte for byte"
    );

    // 发布来源世界运行若干拍，经同修订换根切到编辑来源后保存。
    let plan = ResolvedPlan::mixed(
        &artifacts,
        Window {
            purpose: "probe".into(),
            warm_up_ticks: 0,
            observation_ticks: WARM_TICKS,
        },
    )
    .expect("plan");
    let mut harness = Harness::install(&artifacts, &plan, execution).expect("install harness");
    for _ in 0..WARM_TICKS {
        harness.advance().expect("advance");
    }
    // 只有启用 adapter 特性时 Host 才有第二个变体。
    #[cfg_attr(
        not(feature = "adapter"),
        allow(clippy::infallible_destructuring_match)
    )]
    let world = match &mut harness.world {
        Host::Headless(world) => world,
        #[cfg(feature = "adapter")]
        Host::Adapter(_) => panic!("headless host expected"),
    };
    let origin = *artifacts.revision().canonical_origin();
    let diff_base = EditableDiffBase::try_new(recompiled, origin).expect("diff base");
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(origin),
        LfcaOriginBinding::from_canonical_origin(origin),
        None,
        MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let _events = world
        .cutover_same_revision(
            artifacts.revision().clone(),
            CommittedNetworkSource::Editable {
                state: state.clone(),
                diff_base,
            },
            &descriptor,
            &CutoverPreflightLimits::new(2_147_483_648),
        )
        .expect("switch to editable source");
    let live_vehicles = world.live_vehicles().len();
    let snapshot = encode_lfrs(&world.capture_snapshot().expect("capture"));
    let world_config = artifacts.world_config().expect("world config");
    drop(harness);

    let mut recompile_ms = Vec::with_capacity(samples);
    let mut build_ms = Vec::with_capacity(samples);
    let mut attest_ms = Vec::with_capacity(samples);
    let mut restore_ms = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        let lfca = recompile(&state).expect("host recompile");
        let recompiled_at = Instant::now();
        let revision = build(&lfca);
        let built_at = Instant::now();
        let outcome = attest_editable_shared_network_revision(
            &revision,
            &state,
            recompile,
            FormatLimits::HARD,
            build_options(),
            None,
        );
        let attested_at = Instant::now();
        assert!(
            matches!(
                &outcome,
                AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass
            ),
            "{outcome:?}"
        );
        let diff_base =
            EditableDiffBase::try_new(lfca, *revision.canonical_origin()).expect("diff base");
        let restore_started = Instant::now();
        let restored = restore_lfrs(
            &snapshot,
            revision,
            CommittedNetworkSource::Editable {
                state: state.clone(),
                diff_base,
            },
            world_config,
            execution,
            RESTORE_LIMITS,
        )
        .expect("editable restore");
        let restored_at = Instant::now();
        assert_eq!(restored.world().live_vehicles().len(), live_vehicles);
        drop(restored);
        let ms = |from: Instant, to: Instant| (to - from).as_secs_f64() * 1_000.0;
        recompile_ms.push(ms(started, recompiled_at));
        build_ms.push(ms(recompiled_at, built_at));
        attest_ms.push(ms(built_at, attested_at));
        restore_ms.push(ms(restore_started, restored_at));
    }

    println!("editable load evidence (#529)");
    println!("scale: {}", artifacts.catalog().scale);
    println!("samples: {samples}");
    println!("live vehicles at save: {live_vehicles}");
    println!("LFRE exact bytes: {lfre_bytes} (2 modules)");
    println!("LFCA exact bytes: {}", published_lfca.len());
    println!("LFRS exact bytes: {}", snapshot.len());
    println!(
        "median host recompile ms: {:.3}",
        median_ms(&mut recompile_ms)
    );
    println!("median revision build ms: {:.3}", median_ms(&mut build_ms));
    println!(
        "median editable attestation ms: {:.3}",
        median_ms(&mut attest_ms)
    );
    println!("median restore_lfrs ms: {:.3}", median_ms(&mut restore_ms));
}
