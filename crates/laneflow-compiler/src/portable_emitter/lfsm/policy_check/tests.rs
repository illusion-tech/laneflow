use super::*;

#[test]
fn policy_owner_index_is_granted_within_the_pass_plan_before_allocation() {
    let artifact =
        include_bytes!("../../../../tests/fixtures/portable/lfca-policy-references/expected.lfca");
    let root = preflight_object_values(
        artifact,
        PortableObjectKind::CanonicalArtifact,
        FormatLimits::HARD,
    )
    .unwrap()
    .registry_view();
    let policies = table(root, 2, 23).unwrap();
    let required = u64::from(policies.row_count()) * size_of::<[u8; 16]>() as u64;
    assert!(required > 0);
    let accounting = crate::portable_emitter::budget::test_accounting(u64::MAX);
    let too_small = accounting
        .admit("test-owners", Plan::of(required - 1))
        .unwrap();
    assert_eq!(
        policy_owner_ids(policies, &too_small).err(),
        Some(PortableEmissionError::InternalAccountingDefect {
            point: "test-owners"
        })
    );
    assert_eq!(too_small.in_use(), 0, "授予被拒绝时不分配");
    too_small.finish(0).unwrap();
    let enough = accounting.admit("test-owners", Plan::of(required)).unwrap();
    let owners = policy_owner_ids(policies, &enough).unwrap();
    assert_eq!(owners.len(), policies.row_count() as usize);
    assert_eq!(enough.in_use(), required);
    drop(owners);
    enough.finish(0).unwrap();
    assert_eq!(accounting.ledger.live(), 0);
}
