//! 二分叶的覆盖与逐叶重放：不以计数总和代替完整参数域覆盖。

use super::*;

pub(super) const LIMIT: usize = 65_536;

pub(super) fn verify(
    leaves: &[(u64, u8)],
    first_only: bool,
    reject: impl Fn(Interval, Interval) -> Option<u8>,
) -> bool {
    let mut codes = BTreeSet::new();
    for &(code, reason) in leaves {
        if code == 0 || !codes.insert(code) {
            return false;
        }
        let depth = 63 - code.leading_zeros();
        if depth > DEPTH_LIMIT {
            return false;
        }
        let mut u = Interval { lo: 0.0, hi: 1.0 };
        let mut v = u;
        for bit in (0..depth).rev() {
            let axis = if !first_only && u.hi - u.lo > v.hi - v.lo {
                &mut u
            } else {
                &mut v
            };
            let mid = (axis.lo + axis.hi) * 0.5;
            if (code >> bit) & 1 == 0 {
                axis.hi = mid;
            } else {
                axis.lo = mid;
            }
        }
        if reject(u, v) != Some(reason) {
            return false;
        }
    }
    while let Some(code) = codes.pop_last() {
        if code == 1 {
            return codes.is_empty();
        }
        if code % 2 == 0 || !codes.remove(&(code - 1)) || !codes.insert(code / 2) {
            return false;
        }
    }
    false
}

#[test]
fn certificate_requires_complete_disjoint_cover_and_valid_reasons() {
    assert!(verify(&[(4, 1), (5, 1), (3, 1)], false, |_, _| Some(1)));
    assert!(!verify(&[(4, 1), (3, 1)], false, |_, _| Some(1)));
    assert!(!verify(&[(4, 1), (5, 1), (3, 1), (2, 1)], false, |_, _| {
        Some(1)
    }));
    assert!(!verify(&[(4, 1), (5, 1), (3, 1)], false, |_, _| Some(2)));
    assert!(!verify(&[(1, 1), (1, 1)], true, |_, _| Some(1)));
    assert!(verify(&[(2, 1), (3, 2)], true, |_, v| Some(
        if v.hi == 0.5 { 1 } else { 2 }
    )));
}
