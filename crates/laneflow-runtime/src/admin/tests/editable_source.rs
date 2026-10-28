//! 编辑来源（#1119，`committed-road-editing-state.md`）在安装、快照恢复、同修订换根与
//! 跨修订直移各入口的合同测试。宿主重编译路径由现有编译器、格式与共享静态路网入口组合。

use super::*;
use crate::admin::cutover::tests::transaction_tests::source_for;
use crate::admin::cutover_migration::tests::conflict_cutover_test_module;
use crate::{
    CutoverError, CutoverPreflightLimits, CutoverTransactionLimits, EditableDiffBase,
    EditableDiffBaseMismatch, LfcaOriginBinding, MigrationPolicyKind,
    NetworkRevisionCutoverDescriptor, PolicyPin, RoadEditingState, RoadEditingStateModule,
    SemanticDiffOriginBinding, TrafficWorld, WorldPolicySelection, encode_lfrs,
};
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, PortableDiffBase, PortableEmissionProvenance,
    emit_portable_candidate, road_editing as lfre,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
use laneflow_static_contract::{PortableObjectKind, SEMANTIC_DIFF_FORMAT_VERSION};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const DOCUMENT_KEY: &str = "runtime-live-conflict-cutover.lfre";

fn generous_limits() -> SnapshotRestoreLimits {
    SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024)
}

/// 单模块道路编辑状态：base（`target = false`）或插入前置 passage 的 target。
fn state(target: bool) -> RoadEditingState {
    let limits = CompileLimits::p100_initial_v2();
    let bytes = lfre::RoadEditingSourceWriter::new(&limits)
        .write(conflict_cutover_test_module(target, false, false, false))
        .expect("Road Editing source")
        .as_bytes()
        .to_vec();
    RoadEditingState::try_new(vec![RoadEditingStateModule::new(
        DOCUMENT_KEY.to_owned(),
        bytes,
    )])
    .expect("road editing state")
}

/// 宿主重编译产物：共享路网修订、构建它所用的 exact LFCA、LFSD 及其绑定。
struct Recompiled {
    revision: Arc<SharedNetworkRevision>,
    lfca: Vec<u8>,
    lfsd: Vec<u8>,
    lfsd_binding: SemanticDiffOriginBinding,
}

/// 宿主重编译：逐模块 `LFRE` 准入 → 编译 → 发射 → 后发射检查 → 构建修订。
fn recompile(state: &RoadEditingState, provenance: &str, base: Option<&[u8]>) -> Recompiled {
    let limits = CompileLimits::p100_initial_v2();
    let mut unit = CompilationUnitBuilder::new(limits);
    for module in state.modules() {
        let input = lfre::RoadEditingModuleInput::try_new(
            module.source_document_key(),
            module.source_bytes(),
            None,
        )
        .expect("module input");
        unit.add_road_editing_module(input).expect("admission");
    }
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .unwrap_or_else(|bundle| panic!("compile diagnostics: {:?}", bundle.diagnostics()));
    let provenance = PortableEmissionProvenance::try_new(provenance).expect("provenance");
    let diff_base = match base {
        None => PortableDiffBase::Genesis,
        Some(bytes) => PortableDiffBase::Artifact(
            preflight_object_values(
                bytes,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .expect("base artifact values"),
        ),
    };
    let candidate = emit_portable_candidate(&output, &provenance, FormatLimits::HARD, diff_base)
        .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared revision");
    Recompiled {
        revision,
        lfca: candidate.canonical_artifact().bytes().to_vec(),
        lfsd: candidate.semantic_diff().bytes().to_vec(),
        lfsd_binding: SemanticDiffOriginBinding::new(
            SEMANTIC_DIFF_FORMAT_VERSION,
            candidate.semantic_diff().digest(),
            candidate.semantic_diff().byte_length(),
        ),
    }
}

fn editable(state: &RoadEditingState, compiled: &Recompiled) -> CommittedNetworkSource {
    CommittedNetworkSource::Editable {
        state: state.clone(),
        diff_base: EditableDiffBase::try_new(
            compiled.lfca.clone(),
            *compiled.revision.canonical_origin(),
        )
        .expect("diff base corresponds"),
    }
}

fn install(
    revision: &Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
) -> Result<TrafficWorld, InstallError> {
    TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(4, 4, 64, 8, 100),
        crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        source,
        284,
        WorldPolicySelection::Pinned(PolicyPin {
            policy: revision
                .identity()
                .stable_id(laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(0))
                .expect("fixture policy"),
        }),
    )
}

fn restore(
    bytes: &[u8],
    revision: &Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
) -> Result<RestoredSnapshot, SnapshotRestoreError> {
    restore_lfrs(
        bytes,
        Arc::clone(revision),
        source,
        WorldConfig::new(4, 4, 64, 8, 100),
        crate::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        generous_limits(),
    )
}

fn diff_base_of(world: &TrafficWorld) -> &EditableDiffBase {
    match world.committed_source() {
        CommittedNetworkSource::Editable { diff_base, .. } => diff_base,
        CommittedNetworkSource::Published { .. } => panic!("world must hold an editable source"),
    }
}

fn source_kind_offset(bytes: &[u8]) -> usize {
    let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
    let relative = usize::from(
        root._tab
            .vtable()
            .get(wire::RuntimeSnapshot::VT_SOURCE_KIND),
    );
    assert_ne!(relative, 0, "source_kind must be present");
    root._tab.loc() + relative
}

fn clear_root_field(bytes: &mut [u8], field: laneflow_runtime_snapshot_wire::runtime::VOffsetT) {
    let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
    let table = root._tab.loc();
    let backwards = i32::from_le_bytes(bytes[table..table + 4].try_into().expect("vtable offset"));
    let vtable = table - usize::try_from(backwards).expect("positive vtable offset");
    let entry = vtable + usize::from(field);
    bytes[entry..entry + 2].copy_from_slice(&0_u16.to_le_bytes());
}

#[test]
fn host_recompile_is_byte_reproducible_for_same_build_and_parameters() {
    let state = state(false);
    let first = recompile(&state, "runtime-editable-source-a", None);
    let second = recompile(&state, "runtime-editable-source-a", None);
    assert_eq!(first.lfca, second.lfca);
    assert_eq!(
        *first.revision.canonical_origin(),
        *second.revision.canonical_origin()
    );
}

#[test]
fn diff_base_rejects_bytes_of_another_build() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let b = recompile(&state, "runtime-editable-source-b", None);
    // 发射 provenance 进入 LFCA：同语义修订、不同字节（同修订重发布）。
    assert_eq!(
        a.revision.canonical_origin().network_revision(),
        b.revision.canonical_origin().network_revision()
    );
    assert_ne!(a.lfca, b.lfca);
    assert_eq!(
        EditableDiffBase::try_new(b.lfca.clone(), *a.revision.canonical_origin()),
        Err(EditableDiffBaseMismatch)
    );
    let base = EditableDiffBase::try_new(a.lfca.clone(), *a.revision.canonical_origin())
        .expect("own bytes");
    assert!(base.corresponds_to(*a.revision.canonical_origin()));
    assert!(!base.corresponds_to(*b.revision.canonical_origin()));
    assert_eq!(base.lfca_bytes(), a.lfca.as_slice());
}

#[test]
fn install_requires_byte_exact_diff_base() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let b = recompile(&state, "runtime-editable-source-b", None);

    let world = install(&a.revision, editable(&state, &a)).expect("editable install");
    assert_eq!(diff_base_of(&world).lfca_bytes(), a.lfca.as_slice());
    match world.committed_source() {
        CommittedNetworkSource::Editable { state: held, .. } => assert_eq!(held, &state),
        CommittedNetworkSource::Published { .. } => panic!("editable source expected"),
    }

    // 同修订、同语义的重发布字节不能充当差异基线。
    assert_eq!(
        install(&a.revision, editable(&state, &b)).err(),
        Some(InstallError::EditableDiffBaseMismatch)
    );
}

#[test]
fn editable_snapshot_binds_state_without_copying_or_embedding_it() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let world = install(&a.revision, editable(&state, &a)).expect("editable install");

    let snapshot = world.capture_snapshot().expect("capture");
    match (world.committed_source(), snapshot.source()) {
        (
            CommittedNetworkSource::Editable { state: live, .. },
            CommittedNetworkSource::Editable {
                state: captured, ..
            },
        ) => assert!(std::ptr::eq(
            live.modules().as_ptr(),
            captured.modules().as_ptr()
        )),
        _ => panic!("editable capture expected"),
    }

    let bytes = encode_lfrs(&snapshot);
    let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).expect("verified LFRS");
    assert_eq!(root.source_kind(), wire::SourceKind::Editable);
    assert!(root.source_published().is_none());
    let binding = root.source_editable().expect("editable binding");
    assert_eq!(
        &binding.state_digest().expect("digest").0,
        state.digest().as_bytes()
    );
    assert_eq!(binding.state_byte_length(), state.total_byte_length().get());
    assert_eq!(binding.module_count(), 1);
    let source_bytes = &state.modules()[0].source_bytes()[..64];
    assert!(
        !bytes
            .windows(source_bytes.len())
            .any(|window| window == source_bytes),
        "LFRE bytes must not be embedded"
    );
}

#[test]
fn editable_restore_round_trips_and_rejects_binding_mismatches() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let b = recompile(&state, "runtime-editable-source-b", None);
    let world = install(&a.revision, editable(&state, &a)).expect("editable install");
    let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));

    // 新进程：宿主按摘要取回状态并重编译出新的修订与差异基线。
    let reloaded = RoadEditingState::try_new(state.modules().to_vec()).expect("reload state");
    assert_eq!(reloaded.digest(), state.digest());
    let fresh = recompile(&reloaded, "runtime-editable-source-a", None);
    let restored = restore(&bytes, &fresh.revision, editable(&reloaded, &fresh))
        .expect("editable restore")
        .into_world();
    assert_eq!(diff_base_of(&restored).lfca_bytes(), fresh.lfca.as_slice());

    // 摘要失配：宿主交回的不是快照绑定的那一份状态。
    let mut modules = state.modules().to_vec();
    modules.push(RoadEditingStateModule::new(
        "extra.lfre".to_owned(),
        vec![0; 8],
    ));
    let other = RoadEditingState::try_new(modules).expect("other state");
    assert_eq!(
        restore(&bytes, &a.revision, editable(&other, &a)).err(),
        Some(SnapshotRestoreError::EditableSourceBindingMismatch)
    );

    // 差异基线来自同修订的另一份字节。
    assert_eq!(
        restore(&bytes, &a.revision, editable(&state, &b)).err(),
        Some(SnapshotRestoreError::Install(
            InstallError::EditableDiffBaseMismatch
        ))
    );

    // 目标来源种类与快照不一致。
    let published = source_for(
        *a.revision.canonical_origin(),
        "fixture://editable-as-published",
    );
    assert_eq!(
        restore(&bytes, &a.revision, published.clone()).err(),
        Some(SnapshotRestoreError::SourceKindMismatch)
    );

    // 缺编辑来源表。
    let mut missing = bytes.clone();
    clear_root_field(&mut missing, wire::RuntimeSnapshot::VT_SOURCE_EDITABLE);
    assert_eq!(
        restore(&missing, &a.revision, editable(&state, &a)).err(),
        Some(SnapshotRestoreError::MissingField {
            field: "source_editable"
        })
    );

    // 种类改为 Published 而保留编辑来源表：多余的绑定表。
    let mut as_published = bytes.clone();
    let offset = source_kind_offset(&as_published);
    as_published[offset] = wire::SourceKind::Published.0;
    assert_eq!(
        restore(&as_published, &a.revision, published).err(),
        Some(SnapshotRestoreError::UnexpectedSourceBinding {
            field: "source_editable"
        })
    );

    // 未知来源种类。
    let mut unknown = bytes;
    unknown[offset] = 3;
    assert_eq!(
        restore(&unknown, &a.revision, editable(&state, &a)).err(),
        Some(SnapshotRestoreError::UnsupportedSourceKind { actual: 3 })
    );
}

#[test]
fn published_snapshot_rejects_editable_target_and_extra_binding() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let published = source_for(*a.revision.canonical_origin(), "fixture://published-world");
    let world = install(&a.revision, published).expect("published install");
    let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));

    assert_eq!(
        restore(&bytes, &a.revision, editable(&state, &a)).err(),
        Some(SnapshotRestoreError::SourceKindMismatch)
    );

    let mut as_editable = bytes;
    let offset = source_kind_offset(&as_editable);
    as_editable[offset] = wire::SourceKind::Editable.0;
    assert_eq!(
        restore(&as_editable, &a.revision, editable(&state, &a)).err(),
        Some(SnapshotRestoreError::UnexpectedSourceBinding {
            field: "source_published"
        })
    );
}

fn same_revision(
    world: &mut TrafficWorld,
    target: &Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
) -> Result<(), CutoverError> {
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        None,
        MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    world
        .cutover_same_revision(
            Arc::clone(target),
            source,
            &descriptor,
            &CutoverPreflightLimits::new(1_048_576),
        )
        .map(|_| ())
}

#[test]
fn same_revision_rebase_switches_source_kind_both_ways() {
    let state = state(false);
    let a = recompile(&state, "runtime-editable-source-a", None);
    let b = recompile(&state, "runtime-editable-source-b", None);
    let published = source_for(*a.revision.canonical_origin(), "fixture://published-world");
    let mut world = install(&a.revision, published.clone()).expect("published install");

    // 发布 → 编辑：同修订换根到用重编译 exact LFCA 构建的新根对象。
    let rebuilt = recompile(&state, "runtime-editable-source-a", None);
    same_revision(&mut world, &rebuilt.revision, editable(&state, &rebuilt))
        .expect("published to editable rebase");
    assert_eq!(diff_base_of(&world).lfca_bytes(), rebuilt.lfca.as_slice());

    // 差异基线不是目标根对象的构建字节：拒绝，旧来源原样保留。
    assert_eq!(
        same_revision(&mut world, &rebuilt.revision, editable(&state, &b)),
        Err(CutoverError::TargetEditableDiffBaseMismatch)
    );
    assert_eq!(diff_base_of(&world).lfca_bytes(), rebuilt.lfca.as_slice());

    // 编辑 → 发布：转为 runtime-only，差异基线随旧聚合退休。
    same_revision(&mut world, &rebuilt.revision, published).expect("editable to published");
    assert!(matches!(
        world.committed_source(),
        CommittedNetworkSource::Published { .. }
    ));
}

fn cross_revision(
    world: &mut TrafficWorld,
    target: &Recompiled,
    source: CommittedNetworkSource,
) -> Result<(), CutoverError> {
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*world.revision().canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.revision.canonical_origin()),
        Some(target.lfsd_binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world.prepare_cross_revision_cutover(
        Arc::clone(&target.revision),
        source,
        &descriptor,
        &target.lfsd,
        &CutoverPreflightLimits::new(1_048_576),
        &CutoverTransactionLimits::default(),
    )?;
    transaction.commit(world).map(|_| ())
}

#[test]
fn cross_revision_commit_promotes_target_lfca_as_next_diff_base() {
    let base_state = state(false);
    let target_state = state(true);
    let base = recompile(&base_state, "runtime-editable-source-a", None);
    let target = recompile(&target_state, "runtime-editable-source-a", Some(&base.lfca));
    let republished_target =
        recompile(&target_state, "runtime-editable-source-b", Some(&base.lfca));
    assert_ne!(
        base.revision.canonical_origin().network_revision(),
        target.revision.canonical_origin().network_revision()
    );

    let mut world = install(&base.revision, editable(&base_state, &base)).expect("editable");
    assert_eq!(
        cross_revision(
            &mut world,
            &target,
            editable(&target_state, &republished_target)
        ),
        Err(CutoverError::TargetEditableDiffBaseMismatch)
    );
    assert_eq!(diff_base_of(&world).lfca_bytes(), base.lfca.as_slice());

    cross_revision(&mut world, &target, editable(&target_state, &target))
        .expect("editable cross-revision commit");
    assert_eq!(diff_base_of(&world).lfca_bytes(), target.lfca.as_slice());
    match world.committed_source() {
        CommittedNetworkSource::Editable { state, .. } => assert_eq!(state, &target_state),
        CommittedNetworkSource::Published { .. } => panic!("editable source expected"),
    }
}

#[test]
fn cross_revision_editable_to_published_retires_diff_base() {
    let base_state = state(false);
    let target_state = state(true);
    let base = recompile(&base_state, "runtime-editable-source-a", None);
    let target = recompile(&target_state, "runtime-editable-source-a", Some(&base.lfca));
    let mut world = install(&base.revision, editable(&base_state, &base)).expect("editable");
    cross_revision(
        &mut world,
        &target,
        source_for(
            *target.revision.canonical_origin(),
            "fixture://published-target",
        ),
    )
    .expect("editable to published cross-revision commit");
    assert!(matches!(
        world.committed_source(),
        CommittedNetworkSource::Published { .. }
    ));
}
