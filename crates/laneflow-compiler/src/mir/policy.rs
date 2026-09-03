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
