use issue_308_compiler_budget_calibration_research::{
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, GraphProfileId, load_repository_contract,
    run_identity_fresh_process_pilot,
};
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn identity_pilot_uses_seven_fresh_child_processes() {
    let executable = Path::new(env!(
        "CARGO_BIN_EXE_issue-308-compiler-budget-calibration-research"
    ));
    let trusted = load_repository_contract().expect("frozen contract");
    let report = run_identity_fresh_process_pilot(
        &trusted,
        executable,
        "integration-fresh-process-pilot",
        GraphProfileId::WideStar,
        1,
        None,
    )
    .expect("fresh-process pilot");

    assert_eq!(report.samples.len(), FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    assert!(report.samples.iter().all(|sample| sample.child_pid > 0));
    assert_eq!(
        report
            .samples
            .iter()
            .map(|sample| sample.compiler_instance_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        FRESH_PROCESS_PILOT_SAMPLE_COUNT
    );
    assert!(report.semantic_digest_consistent);
    assert!(report.guard_preflight_evaluated);
    assert_eq!(
        report.guard_preflights.len(),
        FRESH_PROCESS_PILOT_SAMPLE_COUNT
    );
    assert!(
        report
            .guard_preflights
            .iter()
            .all(|guard| guard.allows_child_start && guard.triggers.is_empty())
    );
}
