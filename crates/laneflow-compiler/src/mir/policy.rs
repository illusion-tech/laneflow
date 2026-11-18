//! MIR 的路权声明规范化；源绑定已由 HIR 完成。
use super::*;
use crate::declaration::TypedAstDeclaration;
use crate::policy::model::{PolicyRecord, StreamRule};
type MirPolicy = PolicyRecord<MirManeuverGateKey, MirParticipantStreamKey, MirParticipantClassKey>;
mod access;
mod passages;
mod protected;
mod targets;
mod validation;
mod work;
pub(crate) use validation::validate;

#[cfg(test)]
fn fixture() -> (CompilationUnit, MirUnit) {
    let unit = crate::compiler::policy_tests::unit_with_policy(false, |_, _| {}).unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();
    let mir = super::lower_to_mir(&unit, &hir).unwrap();
    (unit, mir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_work_is_zero_and_all_policies_share_one_budget() {
        let (mut unit, mut mir) = fixture();
        let single = validate(&unit, &mut mir).unwrap();
        assert!(single > 0);
        let (_, second) = fixture();
        let mut policies = std::mem::take(&mut mir.policies).into_vec();
        policies.extend(second.policies);
        mir.policies = policies.into_boxed_slice();
        unit.limits = unit
            .limits
            .with_test_policy_work_limit(single.try_into().unwrap());
        let error = validate(&unit, &mut mir).unwrap_err();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::PolicyValidationWorkUnits,
                ..
            }
        ));
        mir.policies = Box::default();
        unit.limits = unit.limits.with_test_policy_work_limit(0);
        assert_eq!(validate(&unit, &mut mir).unwrap(), 0);
    }
}
