use super::*;
use crate::{SharedNetworkBuildLimits, SpatialBuildOption};

fn budget(limits: SharedNetworkBuildLimits) -> Budget<'static> {
    Budget {
        options: SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, limits),
        retained: 0,
        scratch: 0,
        work: 0,
    }
}

fn rows(keys: &[u32]) -> Vec<Allowed> {
    keys.iter()
        .enumerate()
        .map(|(i, &owner)| Allowed {
            owner,
            class: ParticipantClassOrdinal::from_raw(i as u32),
        })
        .collect()
}

#[test]
fn direct_and_sparse_indexes_preserve_ranges_and_policy_locality() {
    for (domain, keys, dense) in [
        (0, &[][..], false),
        (100, &[][..], false),
        (4, &[0, 1, 2, 3][..], true),
        (5, &[0, 2, 3, 4][..], true),
        (8, &[0, 0, 2, 3, 3, 7][..], true),
        (10, &[4, 5, 9][..], false),
        (100, &[0, 3, 99][..], false),
        (u32::MAX, &[1, u32::MAX - 1][..], false),
    ] {
        let rows = rows(keys);
        let mut budget = budget(SharedNetworkBuildLimits::new(1_024, 0));
        let index = owner_index(&rows, domain, &mut budget).unwrap();
        assert_eq!(matches!(index, PolicyOwnerIndex::Dense(_)), dense);
        assert_eq!(budget.retained, index.retained_logical_bytes());
        assert_eq!(budget.scratch, 0);
        let count = keys
            .iter()
            .enumerate()
            .filter(|(i, key)| *i == 0 || keys[i - 1] != **key)
            .count();
        assert!(index.retained_logical_bytes() <= (count * size_of::<PolicyOwner>()) as u64);
        // 同一定位表借用两份不同策略的局部规则，不能串到另一份规则 payload。
        let cells: Vec<_> = (0..keys.len() * 2).collect();
        for policy_cells in [&cells[..keys.len()], &cells[keys.len()..]] {
            for owner in (0..domain.min(101))
                .chain(keys.iter().copied())
                .chain([domain, u32::MAX])
            {
                let expected: Vec<_> = keys
                    .iter()
                    .zip(policy_cells)
                    .filter_map(|(&key, &cell)| (key == owner).then_some(cell))
                    .collect();
                let actual = index.cells(owner, policy_cells);
                assert_eq!(actual, expected, "domain={domain}, owner={owner}");
                if let Some(start) = keys.iter().position(|&key| key == owner) {
                    assert!(core::ptr::eq(
                        actual,
                        &policy_cells[start..start + expected.len()]
                    ));
                }
            }
        }
    }
}

#[test]
fn representation_choice_uses_actual_backing_cost() {
    let rows = rows(&[0, 2]);
    for (domain, dense, bytes) in [(5, true, 24), (6, false, 24)] {
        let mut budget = budget(SharedNetworkBuildLimits::new(24, 0));
        let index = owner_index(&rows, domain, &mut budget).unwrap();
        assert_eq!(matches!(index, PolicyOwnerIndex::Dense(_)), dense);
        assert_eq!(index.retained_logical_bytes(), bytes);
    }
}

#[test]
fn owner_index_charges_retained_and_work_before_allocation_and_can_retry() {
    for (domain, keys) in [(4, &[0, 2, 3][..]), (100, &[0, 99][..])] {
        let rows = rows(keys);
        let mut measured = budget(SharedNetworkBuildLimits::new(1_024, 0));
        let index = owner_index(&rows, domain, &mut measured).unwrap();
        let retained = index.retained_logical_bytes();
        let work = measured.work;
        let mut too_small = budget(SharedNetworkBuildLimits::new(retained - 1, 0));
        assert!(matches!(
            owner_index(&rows, domain, &mut too_small),
            Err(BuildError::BudgetExceeded {
                structure: BuildStructure::RetainedOutput,
                ..
            })
        ));
        let mut too_little_work =
            budget(SharedNetworkBuildLimits::new(retained, 0).with_max_policy_work(work - 1));
        assert!(matches!(
            owner_index(&rows, domain, &mut too_little_work),
            Err(BuildError::BudgetExceeded {
                structure: BuildStructure::PolicyWork,
                ..
            })
        ));
        assert_eq!(too_little_work.retained, 0);
        let mut exact =
            budget(SharedNetworkBuildLimits::new(retained, 0).with_max_policy_work(work));
        let retry = owner_index(&rows, domain, &mut exact).unwrap();
        assert_eq!(retry.retained_logical_bytes(), retained);
    }
}

#[test]
fn empty_owner_index_needs_no_domain_sized_allocation_or_work() {
    let mut budget = budget(SharedNetworkBuildLimits::new(0, 0).with_max_policy_work(0));
    let index = owner_index(&[], u32::MAX, &mut budget).unwrap();
    assert_eq!(index.retained_logical_bytes(), 0);
    assert_eq!(budget.work, 0);
    assert!(index.cells::<u8>(u32::MAX, &[]).is_empty());
}
