//! #963 页指纹内容回归；不计实际系统页注入或自然失真覆盖。

use super::{PAGE, fingerprint};

fn near_zero_pair(nonzero: usize, cleared: usize) -> (Vec<u8>, Vec<u8>) {
    let mut before = vec![0xa5; 3 * PAGE];
    before[PAGE..2 * PAGE].fill(0);
    before[PAGE..PAGE + cleared].fill(0x33);
    let mut after = before.clone();
    after[PAGE..2 * PAGE].fill(0);
    after[PAGE + 3_000..PAGE + 3_000 + nonzero].fill(0x5a);
    (before, after)
}

#[test]
fn near_zero_requires_both_thresholds() {
    for (nonzero, cleared, expected) in [
        (15, 255, "Partial"),
        (15, 256, "Zero"),
        (15, 257, "Zero"),
        (16, 255, "Partial"),
        (16, 256, "Zero"),
        (16, 257, "Zero"),
        (17, 255, "Partial"),
        (17, 256, "Partial"),
        (17, 257, "Partial"),
    ] {
        let (before, after) = near_zero_pair(nonzero, cleared);
        assert_eq!(
            fingerprint(&before, &after, PAGE),
            expected,
            "nonzero {nonzero}, cleared {cleared}"
        );
    }
}

#[test]
fn near_zero_recognizes_sparse_and_dense_write_loss() {
    let (mut before, after) = near_zero_pair(1, PAGE / 2);
    assert_eq!(fingerprint(&before, &after, PAGE), "Zero");
    before[PAGE..2 * PAGE].fill(0x33);
    assert_eq!(fingerprint(&before, &after, PAGE), "Zero");

    let mut too_many_residuals = after;
    too_many_residuals[PAGE + 3_000..PAGE + 3_017].fill(0x5a);
    assert_eq!(fingerprint(&before, &too_many_residuals, PAGE), "Replaced");
}

#[test]
fn local_clearing_on_sparse_page_remains_partial() {
    let (before, _) = near_zero_pair(0, 40);
    let mut after = before.clone();
    after[PAGE..PAGE + 30].fill(0);
    assert_eq!(fingerprint(&before, &after, PAGE), "Partial");
}

#[test]
fn unchanged_and_full_zero_keep_precedence() {
    let (mut before, _) = near_zero_pair(0, 40);
    assert_eq!(fingerprint(&before, &before, PAGE), "Unchanged");
    let mut after = before.clone();
    after[PAGE..2 * PAGE].fill(0);
    assert_eq!(fingerprint(&before, &after, PAGE), "Zero");
    before[PAGE..2 * PAGE].fill(0);
    assert_eq!(fingerprint(&before, &before, PAGE), "Unchanged");
}

#[test]
fn near_zero_exact_donor_match_precedes_zero_classification() {
    for (donor_page, expected) in [(0, "Shifted(-1)"), (2, "Shifted(1)")] {
        let (mut before, after) = near_zero_pair(16, 256);
        before[donor_page * PAGE..(donor_page + 1) * PAGE].copy_from_slice(&after[PAGE..2 * PAGE]);
        assert_eq!(fingerprint(&before, &after, PAGE), expected);
    }
}
