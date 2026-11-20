#![cfg(feature = "verification-cost-counts")]

use std::num::NonZeroU32;

use laneflow_runtime::ExecutionConfig;
use laneflow_synth_harness::verification_cost::{Mode, run};

#[test]
fn library_counts_rejects_a_process_without_instrumented_global_allocator() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let error = run(
        root.path(),
        &"c".repeat(40),
        ExecutionConfig::new(NonZeroU32::MIN),
        Mode::Counts,
        &output,
    )
    .unwrap_err();
    #[cfg(not(feature = "allocation"))]
    assert!(error.to_string().contains("全局分配器"));
    #[cfg(feature = "allocation")]
    assert!(error.to_string().contains("产品 allocation"));
    assert!(!output.exists());
}
