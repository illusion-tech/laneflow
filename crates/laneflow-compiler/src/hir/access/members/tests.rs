use super::*;
use crate::SourceSpan;
use crate::ledger::{AccountingDefect, Ledger, LedgerStage, PassLabel};
use std::mem::size_of_val;

const LABEL: PassLabel = PassLabel::new(LedgerStage::CompilePipeline, "test-access-members");

fn resolved_classes(pass: &PassBudget<'_>, keys: &[u32]) -> BoundedVec<ResolvedRuleClass> {
    let span = SourceLocation::from(SourceSpan::point("classes".into(), 1, 1));
    pass.collect(
        keys.iter()
            .map(|&key| (HirParticipantClassKey::from_raw(key), span.clone())),
    )
    .unwrap()
}

#[test]
fn access_member_grants_follow_rule_workspaces_and_frozen_rows() {
    let rules = [(&[2, 1, 2, 1][..], 0..2), (&[1, 0][..], 2..4)];
    let mut plan = RuleClassMemberPlan::default();
    for (keys, _) in &rules {
        plan.record_rule(keys.len());
    }
    let admitted = plan.output().plus(plan.scratch());
    let ledger = Ledger::with_budget(admitted.bytes().unwrap());
    let pass = ledger.admit_pass(LABEL, admitted).unwrap();
    let mut staged = plan.stage(&pass).unwrap();
    let staging_bytes = 6 * size_of::<HirAccessRuleParticipantClass>() as u64;
    assert_eq!(pass.in_use(), staging_bytes);

    let mut largest_workspace = 0;
    let mut released_workspaces = 0;
    for (keys, expected_range) in rules {
        let resolved = resolved_classes(&pass, keys);
        let workspace_bytes = size_of_val(&*resolved) as u64;
        assert_eq!(pass.in_use(), staging_bytes + workspace_bytes);
        assert_eq!(
            staged.append_rule(resolved).unwrap().as_usize_range(),
            expected_range
        );
        assert_eq!(pass.in_use(), staging_bytes, "per-rule workspace survived");
        largest_workspace = largest_workspace.max(workspace_bytes);
        released_workspaces += workspace_bytes;
    }

    let rows = staged.freeze(&pass).unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.participant_class.raw())
            .collect::<Vec<_>>(),
        [1, 2, 0, 1]
    );
    let output_bytes = size_of_val(&*rows) as u64;
    assert_eq!(rows.granted_bytes(), output_bytes);
    assert_eq!(pass.in_use(), output_bytes, "raw staging survived freeze");
    pass.finish(output_bytes).unwrap();
    assert_eq!((ledger.live(), ledger.reserved()), (output_bytes, 0));
    drop(rows);

    let report = ledger.close().unwrap();
    assert_eq!(
        report.peak_live,
        staging_bytes + largest_workspace.max(output_bytes)
    );
    assert_eq!(
        report.released,
        staging_bytes + released_workspaces + output_bytes
    );
}

#[test]
fn access_member_plan_matches_observed_peak_when_all_references_survive() {
    // 单条规则且无重复，使逐规则工作区与规范输出都达到原始上界。
    let keys = [2, 0, 1];
    let mut plan = RuleClassMemberPlan::default();
    plan.record_rule(keys.len());
    let planned = plan.output().plus(plan.scratch());
    let ledger = Ledger::with_budget(u64::MAX);
    let pass = ledger.admit_pass(LABEL, planned).unwrap();
    let mut staged = plan.stage(&pass).unwrap();
    staged.append_rule(resolved_classes(&pass, &keys)).unwrap();
    let rows = staged.freeze(&pass).unwrap();
    assert_eq!(rows.len(), keys.len());
    // 查歧义时规范行仍存续，每个成员获一个候选槽，解析工作区与累计暂存已释放。
    let candidates = pass.vec::<AccessCandidate>(rows.len()).unwrap();
    drop(candidates);
    pass.finish(rows.granted_bytes()).unwrap();
    drop(rows);
    let measured = ledger.close().unwrap();
    assert_eq!(planned.bytes(), Some(measured.peak_live));
}

#[test]
fn access_member_underplanned_freeze_reports_an_accounting_defect_and_releases_staging() {
    let mut plan = RuleClassMemberPlan::default();
    plan.record_rule(1);
    plan.record_rule(1);
    let row_bytes = size_of::<HirAccessRuleParticipantClass>() as u64;
    // 两行累计暂存和两行精确输出本应共存，故意少给一个字节。每条规则只用一个解析键。
    let ledger = Ledger::with_budget(u64::MAX);
    let pass = ledger
        .admit_pass(LABEL, Plan::of(4 * row_bytes - 1))
        .unwrap();
    let mut staged = plan.stage(&pass).unwrap();
    for key in [0, 1] {
        staged.append_rule(resolved_classes(&pass, &[key])).unwrap();
    }
    assert_eq!(
        staged.freeze(&pass).unwrap_err(),
        LedgerError::Defect {
            label: LABEL,
            defect: AccountingDefect::GrantExceedsPass
        }
    );
    assert_eq!(pass.in_use(), 0);
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}
