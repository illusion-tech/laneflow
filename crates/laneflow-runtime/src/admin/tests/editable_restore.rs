//! 编辑来源端到端恢复（#529；快照合同 §9「两类来源恢复端到端」的 Editable 半边）。
//! 保存后模拟新进程：宿主按摘要取回状态、重编译出新修订与差异基线、先认证再恢复，
//! 逻辑状态与保存点全等，且继续运行同一 tick 序列逐 tick 摘要相等。

use super::attestation::attest;
use super::*;
use crate::{RouteRegisterInput, TickInput, VehicleSpawnInput, deterministic_state_digest};
use laneflow_static_contract::{ParticipantStreamOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::AttestationVerdict;

const SAVE_PROVENANCE: &str = "runtime-editable-restore";
const DELTA_MS: u64 = 100;
const WARMUP_TICKS: u32 = 8;
const CONTINUE_TICKS: u32 = 24;

/// 编辑来源世界：注册冲突路线并生成一辆车，运行若干 tick。
fn running_world(state: &RoadEditingState, compiled: &Recompiled) -> TrafficWorld {
    let mut world = install(&compiled.revision, editable(state, compiled)).expect("install");
    let stream = compiled
        .revision
        .conflict()
        .participant_stream(ParticipantStreamOrdinal::from_raw(0))
        .expect("participant stream");
    let route = world
        .register_route(RouteRegisterInput::new(
            compiled
                .revision
                .traffic()
                .maneuvers()
                .maneuver_path(stream.maneuver_path())
                .expect("maneuver path")
                .edges()
                .to_vec(),
        ))
        .expect("route");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("spawn");
    let spawned = digest(&world);
    for _ in 0..WARMUP_TICKS {
        world.step(TickInput::new(DELTA_MS)).expect("warmup step");
    }
    // 车辆确实在推进：保存点的逻辑状态不是生成时的状态。
    assert_ne!(digest(&world), spawned);
    world
}

fn digest(world: &TrafficWorld) -> crate::StateDigest {
    deterministic_state_digest(&world.capture_snapshot().expect("capture")).expect("digest")
}

/// 两侧继续运行同一 tick 序列，逐 tick 摘要相等。
fn assert_lockstep(original: &mut TrafficWorld, restored: &mut TrafficWorld) {
    let saved = digest(original);
    for tick in 0..CONTINUE_TICKS {
        original
            .step(TickInput::new(DELTA_MS))
            .expect("original step");
        restored
            .step(TickInput::new(DELTA_MS))
            .expect("restored step");
        assert_eq!(
            digest(original),
            digest(restored),
            "diverged at tick {tick}"
        );
    }
    assert_ne!(digest(original), saved, "continued ticks must change state");
}

#[test]
fn editable_source_restores_after_host_recompile_and_attestation() {
    let state = state(false);
    let compiled = recompile(&state, SAVE_PROVENANCE, None);
    let mut original = running_world(&state, &compiled);
    let saved_digest = digest(&original);
    let bytes = encode_lfrs(&original.capture_snapshot().expect("capture"));

    // 新进程：宿主按摘要取回状态并以同一构建、同一参数重编译。
    let reloaded = RoadEditingState::try_new(state.modules().to_vec()).expect("reload");
    assert_eq!(reloaded.digest(), state.digest());
    let fresh = recompile(&reloaded, SAVE_PROVENANCE, None);
    assert_eq!(fresh.lfca, compiled.lfca);
    // 恢复之前认证将要投入使用的修订（快照合同 §5）。
    assert_eq!(
        attest(&fresh.revision, &reloaded, SAVE_PROVENANCE).verdict,
        AttestationVerdict::Pass
    );
    let mut restored = restore(&bytes, &fresh.revision, editable(&reloaded, &fresh))
        .expect("editable restore")
        .into_world();
    assert_eq!(digest(&restored), saved_digest);
    assert_eq!(diff_base_of(&restored).lfca_bytes(), fresh.lfca.as_slice());
    assert_lockstep(&mut original, &mut restored);
}

/// 跨编译器构建只保证同修订判据：以不同发射参数重编译得到不同字节、同一修订，
/// 恢复仍成立，差异基线随新字节更新（`committed-road-editing-state.md` §4）。
#[test]
fn editable_source_restores_across_builds_with_the_same_revision() {
    let state = state(false);
    let compiled = recompile(&state, SAVE_PROVENANCE, None);
    let mut original = running_world(&state, &compiled);
    let bytes = encode_lfrs(&original.capture_snapshot().expect("capture"));

    let other_build = "runtime-editable-restore-other-build";
    let fresh = recompile(&state, other_build, None);
    assert_ne!(fresh.lfca, compiled.lfca);
    assert_eq!(
        fresh.revision.network_revision(),
        compiled.revision.network_revision()
    );
    assert_eq!(
        attest(&fresh.revision, &state, other_build).verdict,
        AttestationVerdict::Pass
    );
    let mut restored = restore(&bytes, &fresh.revision, editable(&state, &fresh))
        .expect("editable restore across builds")
        .into_world();
    assert_eq!(diff_base_of(&restored).lfca_bytes(), fresh.lfca.as_slice());
    assert_eq!(digest(&restored), digest(&original));
    assert_lockstep(&mut original, &mut restored);
}
