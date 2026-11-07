//! #618 生产库整步 A/B：计时不含建世界、校验、摘要或日志输出。

use std::time::Instant;

use super::support::{Fixtures, Scene};

// 冻结基线为 BLAKE3-256/version 8 摘要；迁移前 SHA-256/version 7 旧值经
// frozen_digest_7_fingerprint 复算核验，差异仅来自算法与版本字段（#1301）。
const CASES: [(u32, u32, bool, usize, &str); 4] = [
    (
        1_000,
        256,
        false,
        128,
        "69c5867fbe65b4227b47aac5bdc02b57d83d028b6f1f2bb96604ba40b37274b3",
    ),
    (
        10_000,
        256,
        false,
        16,
        "a5d521988dd4cbac391a37ba6b4e6efb0bab3e42b33af453ca363656730b72e3",
    ),
    (
        10_000,
        16,
        false,
        16,
        "a62ce81d6ad6d13009b445c1e8549f10486637dd02c8060d6decc1027caee067",
    ),
    (
        10_000,
        256,
        true,
        16,
        "a5d521988dd4cbac391a37ba6b4e6efb0bab3e42b33af453ca363656730b72e3",
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
    // 每进程预先排除两个完整窗口；每个世界另有 fixture 规定的 32 + 8 拍暖机。
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
#[ignore = "证据测量窗口，夜间 evidence.yml 运行（#748）"]
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
