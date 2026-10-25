//! 跨包夹具的隔离与失败语义；夹具失效不能被认证 panic 边界伪装成有效故障证据。

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;
use crate::AttestationReport;
use crate::attestation_fixtures::{flip_lane_length, with_root_comparisons};

fn audit(root: &Arc<SharedNetworkRevision>) -> AttestationReport {
    concluded(attest(
        root,
        FULL_SPATIAL,
        expectation_from(root.canonical_origin()),
        SpatialBuildOption::RetainAvailable,
    ))
}

fn roots() -> (Arc<SharedNetworkRevision>, Arc<SharedNetworkRevision>) {
    let root = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    let mut damaged = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
    flip_lane_length(Arc::get_mut(&mut damaged).unwrap(), 0);
    (root, damaged)
}

#[test]
fn comparison_scope_cleans_up_after_caller_unwind() {
    let (root, damaged) = roots();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            with_root_comparisons(&root, vec![damaged], || panic!("caller unwinds"))
        }))
        .is_err()
    );
    assert_eq!(audit(&root).verdict, AttestationVerdict::Pass);
}

#[test]
fn wrong_target_and_comparison_counts_fail_outside_attestation_catch() {
    for misuse in 0..3 {
        let (root, damaged) = roots();
        let other = build(FULL_SPATIAL, SpatialBuildOption::RetainAvailable);
        let views = match misuse {
            0 => vec![damaged.clone(), damaged],
            1 => vec![damaged],
            // 第一次视图为健康根，只比较一次；多余视图必须拒绝。
            _ => vec![root.clone(), damaged],
        };
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                with_root_comparisons(&root, views, || {
                    audit(if misuse == 0 { &other } else { &root })
                })
            }))
            .is_err()
        );
        assert_eq!(audit(&root).verdict, AttestationVerdict::Pass);
    }
}

#[test]
fn comparison_scope_does_not_affect_another_thread() {
    let (root, damaged) = roots();
    let report = with_root_comparisons(&root, vec![damaged.clone(), damaged], || {
        std::thread::scope(|scope| {
            let independent = scope.spawn(|| audit(&root));
            let injected = audit(&root);
            assert_eq!(
                independent.join().unwrap().verdict,
                AttestationVerdict::Pass
            );
            injected
        })
    });
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(audit(&root).verdict, AttestationVerdict::Pass);
}

#[test]
fn rejected_nested_scope_preserves_outer_injection() {
    let (root, damaged) = roots();
    let report = with_root_comparisons(&root, vec![damaged.clone(), damaged.clone()], || {
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                with_root_comparisons(&root, vec![damaged], || audit(&root))
            }))
            .is_err()
        );
        audit(&root)
    });
    assert_eq!(report.verdict, AttestationVerdict::Mismatch);
    assert_eq!(report.cause, Some(AttestationCause::SharedStaticRoot));
    assert_eq!(audit(&root).verdict, AttestationVerdict::Pass);
}
