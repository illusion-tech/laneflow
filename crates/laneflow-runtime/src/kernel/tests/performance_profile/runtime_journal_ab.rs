//! #618 生产库整步 A/B：计时不含建世界、校验、摘要或日志输出。

use std::time::Instant;

use super::support::{Fixtures, Scene};

const CASES: [(u32, u32, bool, usize, &str); 4] = [
    (
        1_000,
        256,
        false,
        128,
        "31e4896e0811cd527a52b8d7820526ccac2e9cdb70b426da1f1b1814c7a46e1b",
    ),
    (
        10_000,
        256,
        false,
        16,
        "81ebf8b06a078a3eae8859958353fcd8f57c7e93cdcb224785358902fe2cdfcb",
    ),
    (
        10_000,
        16,
        false,
        16,
        "dc65c7b614b506ad5c2ae7ac22391127b055c9b8f1e74426f65db59325ddc5aa",
    ),
    (
        10_000,
        256,
        true,
        16,
        "81ebf8b06a078a3eae8859958353fcd8f57c7e93cdcb224785358902fe2cdfcb",
    ),
];

fn run(case_index: usize, measured_windows: usize) {
    let (count, edges, journal, _, expected_digest) = CASES[case_index];
    let scene = Scene::Lane {
        count,
        edges,
        journal,
    };
    let fixtures = Fixtures::new();
    // 每进程预先排除两个完整窗口；每个世界另有夹具规定的 32 + 8 拍暖机。
    // 正式窗口保留时间顺序，不在 Rust 入口中排序或剔除样本。
    for window in 0..measured_windows + 2 {
        let mut harness = fixtures.world(scene);
        harness.validate();
        let mut samples = vec![0_u128; harness.steps];
        for sample in &mut samples {
            let started = Instant::now();
            harness.step();
            *sample = started.elapsed().as_nanos();
        }
        harness.validate();
        let digest = harness.digest();
        assert_eq!(digest, expected_digest);
        println!(
            "journal-ab {{\"case\":{case_index},\"window\":{window},\"warmup\":{},\"count\":{count},\"edges\":{edges},\"journal\":{journal},\"delta_ms\":{},\"journal_bytes\":{},\"digest\":\"{digest}\",\"step_ns\":{samples:?}}}",
            window < 2,
            harness.delta_ms,
            harness.journal_bytes(),
        );
    }
}

#[test]
fn journal_ab_windows_match_frozen_digests() {
    for case_index in 0..CASES.len() {
        run(case_index, 1);
    }
}

#[test]
#[ignore = "manual production-library release A/B, independent process per case"]
fn runtime_journal_ab_1k_256() {
    run(0, CASES[0].3);
}

#[test]
#[ignore = "manual production-library release A/B, independent process per case"]
fn runtime_journal_ab_10k_256() {
    run(1, CASES[1].3);
}

#[test]
#[ignore = "manual production-library release A/B, independent process per case"]
fn runtime_journal_ab_10k_16() {
    run(2, CASES[2].3);
}

#[test]
#[ignore = "manual production-library release A/B, independent process per case"]
fn runtime_journal_ab_10k_256_armed() {
    run(3, CASES[3].3);
}
