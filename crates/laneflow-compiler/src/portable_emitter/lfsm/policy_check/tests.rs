use super::*;

#[test]
fn policy_owner_index_charges_the_existing_scratch_budget_before_allocation() {
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
    let mut too_small = Scratch::new(required);
    too_small.charge(1).unwrap();
    assert_eq!(
        policy_owner_ids(policies, &mut too_small),
        Err(PortableEmissionError::CompileLimitExceeded {
            dimension: CompileLimitDimension::StageScratchBytes,
            limit: required,
            actual: required + 1,
        })
    );
    assert_eq!(too_small.used(), 1);
    let mut enough = Scratch::new(required + 1);
    enough.charge(1).unwrap();
    let owners = policy_owner_ids(policies, &mut enough).unwrap();
    assert_eq!(owners.len(), policies.row_count() as usize);
    assert_eq!(enough.used(), required + 1);
}
