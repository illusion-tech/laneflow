pub(super) mod base;
mod entity;
mod geometry;
mod policy;
pub(super) mod policy_change;
mod policy_check;
pub use policy_check::check_portable_policy_diff;
mod relation;

use base::{ArtifactIndex, verify_artifact_diff_compatibility};
use entity::{artifact_entity_changes, artifact_static_rule_changes, genesis_entity_changes};
use geometry::{
    artifact_geometry_changes, artifact_spatial_configuration_changes, genesis_geometry_changes,
};
use policy::validate_policy_references;
use relation::{artifact_relation_changes, artifact_relation_tuples, genesis_relation_changes};

use super::relations::{canonical_relation_tuples, entity_stable_id};
use super::*;

fn build_genesis_lfsd(
    output: &CompilationOutput,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<OwnedObject, PortableEmissionError> {
    let target = preflight_object_values(
        artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    let target_index =
        ArtifactIndex::build(target, PortableEmissionError::InternalBindingMismatch)?;
    let policy_changes = policy_change::policy_changes(
        None,
        &target_index,
        output
            .compile_limits()
            .value(CompileLimitDimension::StageScratchBytes),
    )?;
    let entity_changes = genesis_entity_changes(target)?;
    let relation_changes = genesis_relation_changes(output.lir().unit());
    let geometry_changes = genesis_geometry_changes(output.lir().unit(), target)?;
    let spatial_presence = target
        .section(4)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(PortableEmissionError::InternalBindingMismatch)?
        .bytes()
        .to_vec()
        .into_boxed_slice();
    Ok(OwnedObject {
        kind: PortableObjectKind::SemanticDiff,
        sections: vec![
            section(
                1,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U8(0)),
                        field(2, OwnedValue::U16(0)),
                        field(3, OwnedValue::Sha256([0; 32])),
                        field(4, OwnedValue::Sha256([0; 32])),
                        field(5, OwnedValue::U64(0)),
                        field(6, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(
                            7,
                            OwnedValue::Sha256(network_revision.into_digest().into_bytes()),
                        ),
                        field(8, OwnedValue::Sha256(artifact.digest().into_bytes())),
                        field(9, OwnedValue::U64(artifact.byte_length().get())),
                    ])],
                )],
            ),
            section(2, [table(1, entity_changes)]),
            section(3, [table(1, relation_changes)]),
            section(4, [table(1, geometry_changes)]),
            section(5, [table(1, [])]),
            section(
                6,
                [table(
                    1,
                    [row([
                        field(1, OwnedValue::U8(0)),
                        field(3, OwnedValue::Bytes(spatial_presence)),
                    ])],
                )],
            ),
            section(7, [table(1, policy_changes)]),
        ]
        .into_boxed_slice(),
    })
}

pub(super) fn build_lfsd(
    output: &CompilationOutput,
    base: PortableDiffBase<'_>,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<(OwnedObject, ExpectedSemanticDiffBase), PortableEmissionError> {
    match base {
        PortableDiffBase::Genesis => Ok((
            build_genesis_lfsd(output, network_revision, artifact, limits)?,
            ExpectedSemanticDiffBase::Genesis,
        )),
        PortableDiffBase::Artifact(base) => build_artifact_lfsd(
            base,
            network_revision,
            artifact,
            limits,
            output
                .compile_limits()
                .value(CompileLimitDimension::StageScratchBytes),
        ),
    }
}

pub(super) fn verify_target_relation_projection(
    output: &CompilationOutput,
    target: RegistryCheckedObjectView<'_>,
) -> Result<(), PortableEmissionError> {
    let target_index =
        ArtifactIndex::build(target, PortableEmissionError::InternalBindingMismatch)?;
    validate_policy_references(
        &target_index,
        output
            .compile_limits()
            .value(CompileLimitDimension::StageScratchBytes),
        PortableEmissionError::InternalBindingMismatch,
    )?;
    if artifact_relation_tuples(
        &target_index,
        PortableEmissionError::InternalBindingMismatch,
    )? != canonical_relation_tuples(output.lir().unit())
    {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    Ok(())
}

fn build_artifact_lfsd(
    base: ValueCheckedObjectView<'_>,
    target_network_revision: NetworkRevisionId,
    target_artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    policy_scratch_limit: u64,
) -> Result<(OwnedObject, ExpectedSemanticDiffBase), PortableEmissionError> {
    if base.kind() != PortableObjectKind::CanonicalArtifact {
        return Err(PortableEmissionError::InvalidDiffBaseKind);
    }
    let base =
        preflight_object_values(base.bytes(), PortableObjectKind::CanonicalArtifact, limits)?;
    let base_view = base.registry_view();
    let target_view = preflight_object_values(
        target_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    let base_index =
        ArtifactIndex::build(base_view, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_index =
        ArtifactIndex::build(target_view, PortableEmissionError::InternalBindingMismatch)?;
    validate_policy_references(
        &base_index,
        policy_scratch_limit,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    validate_policy_references(
        &target_index,
        policy_scratch_limit,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    verify_artifact_diff_compatibility(base_view, target_view, &base_index, &target_index)?;

    let base_network_revision = network_revision_from_checked(base)?;
    let base_digest = sha256(base.bytes());
    let base_length = ExactByteLength::new(
        u64::try_from(base.bytes().len()).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
    );
    let entity_changes = artifact_entity_changes(&base_index, &target_index)?;
    let relation_changes = artifact_relation_changes(&base_index, &target_index)?;
    let geometry_changes = artifact_geometry_changes(&base_index, &target_index)?;
    let static_rule_changes = artifact_static_rule_changes(&base_index, &target_index)?;
    let spatial_changes = artifact_spatial_configuration_changes(&base_index, &target_index)?;
    let policy_changes =
        policy_change::policy_changes(Some(&base_index), &target_index, policy_scratch_limit)?;

    let expected_base = ExpectedSemanticDiffBase::Artifact {
        network_revision_derivation_version: NETWORK_REVISION_DERIVATION_VERSION,
        network_revision: base_network_revision,
        digest: base_digest,
        byte_length: base_length,
    };

    Ok((
        OwnedObject {
            kind: PortableObjectKind::SemanticDiff,
            sections: vec![
                section(
                    1,
                    [table(
                        1,
                        [row([
                            field(1, OwnedValue::U8(1)),
                            field(2, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                            field(
                                3,
                                OwnedValue::Sha256(
                                    base_network_revision.into_digest().into_bytes(),
                                ),
                            ),
                            field(4, OwnedValue::Sha256(base_digest.into_bytes())),
                            field(5, OwnedValue::U64(base_length.get())),
                            field(6, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                            field(
                                7,
                                OwnedValue::Sha256(
                                    target_network_revision.into_digest().into_bytes(),
                                ),
                            ),
                            field(8, OwnedValue::Sha256(target_artifact.digest().into_bytes())),
                            field(9, OwnedValue::U64(target_artifact.byte_length().get())),
                        ])],
                    )],
                ),
                section(2, [table(1, entity_changes)]),
                section(3, [table(1, relation_changes)]),
                section(4, [table(1, geometry_changes)]),
                section(5, [table(1, static_rule_changes)]),
                section(6, [table(1, spatial_changes)]),
                section(7, [table(1, policy_changes)]),
            ]
            .into_boxed_slice(),
        },
        expected_base,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use laneflow_format::{
        FormatError, FormatLimitConfig, LimitDimension, preflight_object_values,
    };

    const EXPECTED_LFCA: &[u8] =
        include_bytes!("../../tests/fixtures/portable/lfca-full-spatial/expected.lfca");

    #[test]
    fn artifact_diff_reapplies_caller_limits_before_building_base_index() {
        let mut duplicate_stable_id = EXPECTED_LFCA.to_vec();
        let (first_stable_id, second_stable_id) = {
            let view = preflight_object_values(
                &duplicate_stable_id,
                PortableObjectKind::CanonicalArtifact,
                FormatLimits::HARD,
            )
            .unwrap()
            .registry_view();
            let identities = view.section(1).unwrap().table(0).unwrap();
            let value_range = |row_ordinal| {
                let value = identities
                    .row(row_ordinal)
                    .unwrap()
                    .field_by_tag(3)
                    .unwrap()
                    .value_bytes();
                let start = value.as_ptr() as usize - duplicate_stable_id.as_ptr() as usize;
                start..start + value.len()
            };
            (value_range(0), value_range(1))
        };
        let value = duplicate_stable_id[first_stable_id].to_vec();
        let changed_at = second_stable_id.start;
        duplicate_stable_id[second_stable_id].copy_from_slice(&value);
        crate::compiler::portable_fixture_tests::refresh_portable_chunk_digest_containing(
            &mut duplicate_stable_id,
            PortableObjectKind::CanonicalArtifact,
            changed_at,
        );
        let base = preflight_object_values(
            &duplicate_stable_id,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap();
        let target = close_object(EXPECTED_LFCA.to_vec().into_boxed_slice());
        let mut config = FormatLimitConfig::HARD;
        config.max_object_bytes = duplicate_stable_id.len() as u64 - 1;
        let limits = FormatLimits::try_new(config).unwrap();

        assert_eq!(
            build_artifact_lfsd(
                base,
                NetworkRevisionId::from_digest(Sha256Digest::ZERO),
                &target,
                limits,
                u64::MAX,
            ),
            Err(PortableEmissionError::Format(FormatError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                actual: duplicate_stable_id.len() as u64,
                limit: duplicate_stable_id.len() as u64 - 1,
            }))
        );
    }
}
