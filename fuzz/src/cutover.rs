use std::sync::{Arc, OnceLock};

use laneflow_format::{without_digest_probe, ExpectedSemanticDiffBase};
use laneflow_runtime::{
    encode_lfrs, CutoverPreflightLimits, CutoverTransactionLimits, LfcaOriginBinding,
    MigrationPolicyKind, NetworkRevisionCutoverDescriptor, SemanticDiffOriginBinding, TickInput,
    TrafficWorld, WorldBinding,
};
use laneflow_static_contract::{
    ExactByteLength, Sha256Digest, NETWORK_REVISION_DERIVATION_VERSION,
    SEMANTIC_DIFF_FORMAT_VERSION,
};
use laneflow_static_network::SharedNetworkRevision;
use sha2::{Digest, Sha256};

use crate::{mutate, portable::repair_seed_chunk_digests, snapshot, MAX_INPUT_BYTES};

const BASE: &[u8] = include_bytes!(
    "../../crates/laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
);
const TARGET: &[u8] = include_bytes!(
    "../../crates/laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-target.lfca"
);
const DIFF: &[u8] = include_bytes!(
    "../../crates/laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-expected.lfsd"
);

struct Fixtures {
    base: Arc<SharedNetworkRevision>,
    same: Arc<SharedNetworkRevision>,
    target: Arc<SharedNetworkRevision>,
}

fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| Fixtures {
        base: snapshot::revision(BASE),
        same: snapshot::revision(BASE),
        target: snapshot::revision(TARGET),
    })
}

pub(crate) fn expected_base() -> ExpectedSemanticDiffBase {
    let origin = fixtures().base.canonical_origin();
    ExpectedSemanticDiffBase::Artifact {
        network_revision_derivation_version: NETWORK_REVISION_DERIVATION_VERSION,
        network_revision: origin.network_revision(),
        digest: origin.canonical_artifact_digest(),
        byte_length: origin.canonical_artifact_byte_length(),
    }
}

fn binding(bytes: &[u8]) -> SemanticDiffOriginBinding {
    SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
        ExactByteLength::new(bytes.len() as u64),
    )
}

fn capture(world: &TrafficWorld) -> Vec<u8> {
    encode_lfrs(&world.capture_snapshot().expect("capture live world"))
}

fn old_world() -> TrafficWorld {
    let mut world = snapshot::world(&fixtures().base, snapshot::published(&fixtures().base));
    snapshot::populate(&mut world, false);
    world
}

fn descriptor(
    world: &TrafficWorld,
    diff: SemanticDiffOriginBinding,
) -> NetworkRevisionCutoverDescriptor {
    NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*fixtures().base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*fixtures().target.canonical_origin()),
        Some(diff),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    )
}

fn preflight_limits() -> CutoverPreflightLimits {
    CutoverPreflightLimits::new(MAX_INPUT_BYTES as u64)
}

fn transaction_limits() -> CutoverTransactionLimits {
    CutoverTransactionLimits {
        max_journal_bytes: 131_072,
        max_catch_up_lag_ticks: 8,
        max_records_per_pump: 64,
    }
}

fn assert_unchanged(world: &TrafficWorld, before: &[u8], binding: WorldBinding) {
    assert_eq!(
        capture(world),
        before,
        "failed cutover changed captured state/source"
    );
    assert_eq!(
        world.world_binding(),
        binding,
        "failed cutover changed cursors/generation"
    );
    assert!(
        Arc::ptr_eq(&world.revision(), &fixtures().base),
        "failed cutover published a root"
    );
}

fn retry_after_failure(world: &mut TrafficWorld) {
    let before = capture(world);
    let baseline = world.world_binding();
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&fixtures().target),
            snapshot::published(&fixtures().target),
            &descriptor(world, binding(DIFF)),
            DIFF,
            &preflight_limits(),
            &transaction_limits(),
        )
        .expect("failed input must not leave an armed transaction");
    transaction.abandon(world).expect("abandon retry");
    assert_unchanged(world, &before, baseline);
}

fn check_same_revision(data: &[u8]) {
    let flags = data.get(1).copied().unwrap_or(0) & 31;
    let mut world = old_world();
    let before = capture(&world);
    let baseline = world.world_binding();
    let target_binding = if flags & 1 == 0 {
        &fixtures().same
    } else {
        &fixtures().target
    };
    let source_root = if flags & 8 == 0 {
        &fixtures().same
    } else {
        &fixtures().target
    };
    let world_binding = if flags & 16 == 0 {
        baseline
    } else {
        WorldBinding::new(
            537,
            baseline.world_generation(),
            baseline.baseline_command_cursor(),
            baseline.baseline_event_cursor(),
        )
    };
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*fixtures().base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target_binding.canonical_origin()),
        (flags & 2 != 0).then(|| binding(DIFF)),
        if flags & 4 == 0 {
            MigrationPolicyKind::SameRevisionRestore
        } else {
            MigrationPolicyKind::CrossRevisionDirect
        },
        world_binding,
    );
    let result = world.cutover_same_revision(
        Arc::clone(&fixtures().same),
        snapshot::published(source_root),
        &descriptor,
        &preflight_limits(),
    );
    if flags == 0 {
        let _events = result.expect("valid same revision input");
        assert!(Arc::ptr_eq(&world.revision(), &fixtures().same));
        assert_eq!(
            world.world_generation().get(),
            baseline.world_generation().get() + 1
        );
    } else {
        assert!(result.is_err(), "invalid same revision input must reject");
        assert_unchanged(&world, &before, baseline);
    }
}

/// 变异 LFSD 与描述符绑定；失败核对旧根、来源、完整快照和双游标，随后验证可重试。
///
/// # Panics
///
/// 合法迁移种子不能提交、非法绑定被接受或失败产生部分发布时触发 oracle。
pub fn check_cutover(data: &[u8]) {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }
    let selector = data.first().copied().unwrap_or(0);
    let mode = selector & 7;
    if mode == 7 {
        check_same_revision(data);
        return;
    }
    let mut world = old_world();
    let before = capture(&world);
    let baseline = world.world_binding();
    let payload = data.get(1..).unwrap_or_default();
    let mut bytes = if mode == 1 {
        payload.to_vec()
    } else {
        mutate(DIFF, payload)
    };
    if selector & 8 != 0 {
        repair_seed_chunk_digests(DIFF, &mut bytes);
    }
    // 重新认证模式进入 LFSD 解析；原绑定模式独立覆盖长度/摘要损坏拒绝。
    let diff_binding = binding(if mode == 2 { DIFF } else { &bytes });
    let mut descriptor = descriptor(&world, diff_binding);
    if mode == 3 {
        descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*fixtures().target.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*fixtures().base.canonical_origin()),
            Some(diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            baseline,
        );
    } else if mode == 5 {
        descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*fixtures().base.canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*fixtures().target.canonical_origin()),
            Some(diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            WorldBinding::new(
                537,
                baseline.world_generation(),
                baseline.baseline_command_cursor(),
                baseline.baseline_event_cursor(),
            ),
        );
    }
    let target_source = snapshot::published(if mode == 4 {
        &fixtures().base
    } else {
        &fixtures().target
    });
    let limits = if mode == 6 {
        CutoverPreflightLimits::new(0)
    } else {
        preflight_limits()
    };
    let result = without_digest_probe(|| {
        world.prepare_cross_revision_cutover(
            Arc::clone(&fixtures().target),
            target_source,
            &descriptor,
            &bytes,
            &limits,
            &transaction_limits(),
        )
    });
    let must_succeed = mode == 0 && bytes == DIFF;
    if (3..=5).contains(&mode) || (mode == 6 && !bytes.is_empty()) {
        assert!(result.is_err(), "invalid binding/limit must reject");
    }
    match result {
        Err(_) => {
            assert!(!must_succeed, "valid cross revision input must prepare");
            assert_unchanged(&world, &before, baseline);
            retry_after_failure(&mut world);
        }
        Ok(mut transaction) => {
            assert_unchanged(&world, &before, baseline);
            if selector & 16 != 0 {
                world.step(TickInput::new(100)).expect("old world tick");
            }
            let before_commit = capture(&world);
            let commit_binding = world.world_binding();
            match transaction.pump(&mut world) {
                Err(_) => {
                    assert!(!must_succeed, "valid transaction must catch up");
                    assert_unchanged(&world, &before_commit, commit_binding);
                    retry_after_failure(&mut world);
                }
                Ok(_) => match transaction.commit(&mut world) {
                    Err(_) => {
                        assert!(!must_succeed, "valid transaction must commit");
                        assert_unchanged(&world, &before_commit, commit_binding);
                        retry_after_failure(&mut world);
                    }
                    Ok(commit) => {
                        assert!(Arc::ptr_eq(&world.revision(), &fixtures().target));
                        assert_eq!(
                            world.committed_source(),
                            &snapshot::published(&fixtures().target)
                        );
                        assert_eq!(
                            commit.world_generation.get(),
                            baseline.world_generation().get() + 1
                        );
                        assert!(!commit.events.is_empty(), "committed cutover emits events");
                        world
                            .capture_snapshot()
                            .expect("committed world remains capturable");
                    }
                },
            }
        }
    }
}

pub(crate) fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let mut raw = vec![1];
    raw.extend_from_slice(DIFF);
    vec![
        ("cutover_inputs", "cross-revision", vec![0]),
        ("cutover_inputs", "catch-up", vec![16]),
        ("cutover_inputs", "repair-chunks", vec![8]),
        ("cutover_inputs", "raw-lfsd", raw),
        ("cutover_inputs", "stale-binding", vec![2, 1, 40, 0, 1, 0]),
        ("cutover_inputs", "swapped-origins", vec![3]),
        ("cutover_inputs", "wrong-source", vec![4]),
        ("cutover_inputs", "wrong-world", vec![5]),
        ("cutover_inputs", "over-limit", vec![6]),
        ("cutover_inputs", "same-revision", vec![7, 0]),
        ("cutover_inputs", "same-revision-invalid", vec![7, 31]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populated_world_covers_each_descriptor_control_and_same_revision_rejection() {
        for selector in 0..32 {
            check_cutover(&[selector]);
            check_cutover(&[7, selector]);
        }
    }
}
