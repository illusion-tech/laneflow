use super::*;

const SINGLE: PassLabel = PassLabel::new(LedgerStage::SingleEmission, "test-single");
const SUB: PassLabel = PassLabel::new(LedgerStage::SingleEmission, "test-sub");
const CHECK: PassLabel = PassLabel::new(LedgerStage::PublicCheck, "test-check");

fn assert_conserved(ledger: &Ledger) {
    let state = lock(&ledger.state);
    assert!(
        state.live + state.reserved <= state.budget,
        "L + R ≤ B 被破坏"
    );
    for frame in &state.frames {
        assert!(frame.in_use + frame.carved <= frame.plan, "U ≤ P 被破坏");
    }
    let unconsumed = state.frames.iter().map(Frame::unconsumed).sum::<u64>();
    assert_eq!(
        state.reserved, unconsumed,
        "R 应等于各活动编译遍未消费额度之和"
    );
}

#[test]
fn admission_accepts_exact_budget_and_rejects_one_more() {
    let ledger = Ledger::with_budget(1024);
    let pass = ledger.admit_pass(SINGLE, 1024).unwrap();
    assert_eq!(ledger.reserved(), 1024);
    assert_conserved(&ledger);
    pass.finish(0).unwrap();
    assert_eq!(ledger.reserved(), 0);

    let rejected = ledger.admit_pass(SINGLE, 1025).err().unwrap();
    assert_eq!(
        rejected,
        LedgerError::BudgetExceeded {
            label: SINGLE,
            requested: 1025,
            limit: 1024,
        }
    );
    assert_eq!((ledger.live(), ledger.reserved()), (0, 0));
    ledger.close().unwrap();
}

#[test]
fn admission_counts_live_output_of_earlier_passes() {
    let ledger = Ledger::with_budget(1000);
    let pass = ledger.admit_pass(SINGLE, 600).unwrap();
    let output = pass.vec::<u8>(600).unwrap();
    pass.finish(output.granted_bytes()).unwrap();
    assert_eq!((ledger.live(), ledger.reserved()), (600, 0));

    let rejected = ledger.admit_pass(CHECK, 401).err().unwrap();
    assert!(matches!(
        rejected,
        LedgerError::BudgetExceeded {
            requested: 1001,
            ..
        }
    ));
    ledger.admit_pass(CHECK, 400).unwrap().finish(0).unwrap();
    drop(output);
    let report = ledger.close().unwrap();
    assert_eq!(report.released, 600);
    assert_eq!(report.peak_live, 600);
}

#[test]
fn unrepresentable_plan_is_a_budget_rejection() {
    let ledger = Ledger::with_budget(u64::MAX);
    let first = ledger.admit_pass(SINGLE, 1).unwrap();
    let kept = first.vec::<u8>(1).unwrap();
    first.finish(1).unwrap();
    let overflow = ledger.admit_pass(SINGLE, u64::MAX).err().unwrap();
    assert_eq!(
        overflow,
        LedgerError::BudgetExceeded {
            label: SINGLE,
            requested: u64::MAX,
            limit: u64::MAX,
        }
    );
    drop(kept);

    let pass = ledger.admit_pass(SINGLE, 16).unwrap();
    let huge = pass.vec::<u64>(usize::MAX).err().unwrap();
    assert!(matches!(
        huge,
        LedgerError::BudgetExceeded {
            requested: u64::MAX,
            ..
        }
    ));
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}

#[test]
fn second_top_level_pass_is_a_defect() {
    let ledger = Ledger::with_budget(100);
    let pass = ledger.admit_pass(SINGLE, 10).unwrap();
    let nested = ledger.admit_pass(CHECK, 10).err().unwrap();
    assert_eq!(
        nested,
        LedgerError::Defect {
            label: CHECK,
            defect: AccountingDefect::NestedTopLevelPass,
        }
    );
    assert_conserved(&ledger);
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}

#[test]
fn grant_beyond_pass_plan_is_a_defect_without_allocation() {
    let ledger = Ledger::with_budget(100);
    let pass = ledger.admit_pass(SINGLE, 8).unwrap();
    let err = pass.vec::<u32>(3).err().unwrap();
    assert_eq!(
        err,
        LedgerError::Defect {
            label: SINGLE,
            defect: AccountingDefect::GrantExceedsPass,
        }
    );
    assert_eq!((ledger.live(), pass.in_use()), (0, 0));
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}

#[test]
fn released_quota_can_be_granted_again_and_peak_is_in_use_not_total() {
    let ledger = Ledger::with_budget(512);
    let pass = ledger.admit_pass(SINGLE, 512).unwrap();
    for _ in 0..3 {
        let scratch = pass.vec::<u8>(512).unwrap();
        assert_eq!(pass.in_use(), 512);
        assert_conserved(&ledger);
        drop(scratch);
        assert_eq!(pass.in_use(), 0);
        assert_eq!(ledger.reserved(), 512);
    }
    pass.finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.stage_peak(LedgerStage::SingleEmission), 512);
    assert_eq!(report.peak_live, 512);
    assert_eq!(report.released, 3 * 512);
}

#[test]
fn nested_sub_pass_draws_from_parent_and_returns_output_to_parent() {
    let ledger = Ledger::with_budget(1000);
    let parent = ledger.admit_pass(SINGLE, 800).unwrap();
    let parent_scratch = parent.vec::<u8>(100).unwrap();

    let too_large = parent.admit_sub(SUB, 701).err().unwrap();
    assert_eq!(
        too_large,
        LedgerError::Defect {
            label: SINGLE,
            defect: AccountingDefect::SubPlanExceedsParent,
        }
    );

    let reserved_before = ledger.reserved();
    let child = parent.admit_sub(SUB, 500).unwrap();
    assert_eq!(ledger.reserved(), reserved_before, "划出子计划不改变 R");
    assert_conserved(&ledger);
    let grandchild = child.admit_sub(SUB, 200).unwrap();
    let deep = grandchild.vec::<u8>(200).unwrap();
    assert_conserved(&ledger);
    grandchild.finish(200).unwrap();
    assert_eq!(child.in_use(), 200, "孙编译遍输出转为子编译遍在用量");
    let child_scratch = child.vec::<u8>(300).unwrap();
    assert_conserved(&ledger);
    drop(child_scratch);
    child.finish(200).unwrap();
    assert_eq!(parent.in_use(), 300);
    assert_conserved(&ledger);

    // 子编译遍输出在父编译遍名下释放时，额度回到父编译遍。
    drop(deep);
    assert_eq!(parent.in_use(), 100);
    assert_eq!(ledger.reserved(), 700);
    drop(parent_scratch);
    parent.finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.stage_peak(LedgerStage::SingleEmission), 600);
    assert_eq!(report.peak_live, 600);
}

#[test]
fn allocator_rejection_rolls_back_the_grant() {
    let ledger = Ledger::with_budget(u64::MAX);
    let capacity = usize::try_from(isize::MAX).unwrap();
    let pass = ledger.admit_pass(SINGLE, capacity as u64).unwrap();
    let err = pass.vec::<u8>(capacity).err().unwrap();
    assert_eq!(err, LedgerError::AllocationFailure { label: SINGLE });
    assert_eq!((ledger.live(), pass.in_use()), (0, 0));
    assert_conserved(&ledger);
    pass.finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.released, 0, "回滚不记为已释放");
}

#[test]
fn early_return_without_finish_returns_unconsumed_reserve() {
    fn failing_pass(ledger: &Ledger) -> Result<(), LedgerError> {
        let pass = ledger.admit_pass(SINGLE, 256)?;
        let child = pass.admit_sub(SUB, 128)?;
        let _scratch = child.vec::<u8>(64)?;
        let _more = pass.vec::<u8>(64)?;
        Err(LedgerError::AllocationFailure { label: SUB })
    }

    let ledger = Ledger::with_budget(256);
    assert!(failing_pass(&ledger).is_err());
    assert_eq!((ledger.live(), ledger.reserved()), (0, 0));
    assert!(lock(&ledger.state).frames.is_empty());
    // 同一账本可以立即准入下一编译遍。
    ledger.admit_pass(SINGLE, 256).unwrap().finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.released, 128);
}

#[test]
fn storage_surviving_a_dropped_pass_returns_quota_when_released() {
    let ledger = Ledger::with_budget(100);
    let kept = {
        let pass = ledger.admit_pass(SINGLE, 100).unwrap();
        pass.vec::<u8>(40).unwrap()
    };
    assert_eq!((ledger.live(), ledger.reserved()), (40, 0));
    drop(kept);
    ledger.close().unwrap();
}

#[test]
fn finish_with_unreturned_grant_is_a_defect() {
    let ledger = Ledger::with_budget(100);
    let pass = ledger.admit_pass(SINGLE, 100).unwrap();
    let leaked = pass.vec::<u8>(30).unwrap();
    let err = pass.finish(0).unwrap_err();
    assert_eq!(
        err,
        LedgerError::Defect {
            label: SINGLE,
            defect: AccountingDefect::UnreturnedGrant,
        }
    );
    assert!(matches!(
        Ledger::close(ledger_with_live(&leaked)),
        Err(LedgerError::Defect {
            defect: AccountingDefect::LedgerNotSettled,
            ..
        })
    ));
    drop(leaked);
    ledger.close().unwrap();
}

/// 构造一份与 `vec` 共享状态的账本句柄，用来在存储仍存活时检查结束条件。
fn ledger_with_live<T>(vec: &BoundedVec<T>) -> Ledger {
    Ledger {
        state: Arc::clone(vec.grant.state.as_ref().unwrap()),
    }
}

#[test]
fn ok_and_err_return_points_transfer_to_host() {
    let ledger = Ledger::with_budget(1000);
    let pass = ledger.admit_pass(SINGLE, 1000).unwrap();
    let mut candidate = pass.vec::<u16>(100).unwrap();
    for value in 0..100 {
        candidate.push(value).unwrap();
    }
    let diagnostics = pass.vec::<u8>(50).unwrap();
    let scratch = pass.vec::<u8>(300).unwrap();
    drop(scratch);
    pass.finish(candidate.granted_bytes() + diagnostics.granted_bytes())
        .unwrap();

    let returned = candidate.transfer_to_host();
    let failure = diagnostics.transfer_to_host();
    let report = ledger.close().unwrap();
    assert_eq!(report.transferred, 250);
    assert_eq!(report.released, 300);
    assert_eq!(report.peak_live, 550);
    // 转交后的存储不受账本约束，宿主仍可读取。
    assert_eq!(returned.len(), 100);
    assert!(failure.is_empty());
}

#[test]
fn writes_beyond_granted_capacity_are_defects() {
    let ledger = Ledger::with_budget(100);
    let pass = ledger.admit_pass(SINGLE, 100).unwrap();
    let mut bytes = pass.vec::<u8>(4).unwrap();
    bytes.extend_from_slice(&[1, 2, 3]).unwrap();
    let overflow = bytes.extend_from_slice(&[4, 5]).unwrap_err();
    assert_eq!(
        overflow,
        LedgerError::Defect {
            label: SINGLE,
            defect: AccountingDefect::CapacityExceeded,
        }
    );
    assert_eq!(&*bytes, &[1, 2, 3], "超出容量时整体不写入");
    bytes.push(4).unwrap();
    assert!(bytes.push(5).is_err());
    bytes.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(&*bytes, &[4, 3, 2, 1]);
    assert_eq!(bytes.capacity(), 4);
    drop(bytes);
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}

#[test]
fn zero_sized_requests_need_no_quota() {
    let ledger = Ledger::with_budget(0);
    let pass = ledger.admit_pass(SINGLE, 0).unwrap();
    let mut empty = pass.vec::<u64>(0).unwrap();
    assert!(empty.push(1).is_err());
    let mut units = pass.vec::<()>(3).unwrap();
    units.push(()).unwrap();
    assert_eq!(units.granted_bytes(), 0);
    drop((empty, units));
    pass.finish(0).unwrap();
    ledger.close().unwrap();
}

#[test]
fn stage_peaks_are_tracked_per_top_level_stage() {
    let ledger = Ledger::with_budget(1000);
    let single = ledger.admit_pass(SINGLE, 400).unwrap();
    drop(single.vec::<u8>(400).unwrap());
    single.finish(0).unwrap();
    let check = ledger.admit_pass(CHECK, 100).unwrap();
    drop(check.vec::<u8>(90).unwrap());
    check.finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.stage_peak(LedgerStage::SingleEmission), 400);
    assert_eq!(report.stage_peak(LedgerStage::PublicCheck), 90);
    assert_eq!(report.stage_peak(LedgerStage::DiffEmission), 0);
}

#[test]
fn granted_storage_moves_across_threads_without_recharging() {
    let ledger = Ledger::with_budget(64);
    let pass = ledger.admit_pass(SINGLE, 64).unwrap();
    let mut bytes = pass.vec::<u8>(64).unwrap();
    bytes.push(7).unwrap();
    let bytes = std::thread::spawn(move || bytes).join().unwrap();
    assert_eq!(ledger.live(), 64);
    drop(bytes);
    pass.finish(0).unwrap();
    let report = ledger.close().unwrap();
    assert_eq!(report.released, 64);
}

#[test]
fn ledger_budget_is_the_profile_host_budget() {
    let limits = CompileLimits::p100();
    let budget = limits.value(CompileLimitDimension::CompilerControlledLiveBytes);
    let ledger = Ledger::new(&limits);
    assert!(ledger.admit_pass(SINGLE, budget + 1).is_err());
    ledger
        .admit_pass(SINGLE, budget)
        .unwrap()
        .finish(0)
        .unwrap();
    ledger.close().unwrap();
}
