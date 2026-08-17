mod base;
mod entity;
mod geometry;
mod relation;

use base::{ArtifactIndex, verify_artifact_diff_compatibility};
use entity::{artifact_entity_changes, artifact_static_rule_changes, genesis_entity_changes};
use geometry::{
    artifact_geometry_changes, artifact_spatial_configuration_changes, genesis_geometry_changes,
};
use relation::{artifact_relation_changes, artifact_relation_tuples, genesis_relation_changes};

use super::relations::{canonical_relation_tuples, entity_stable_id};
use super::*;

fn build_genesis_lfsd(
    output: &CompilationOutput,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
) -> Result<OwnedObject, PortableEmissionError> {
    let target = preflight_object_values_v1(
        artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
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
) -> Result<OwnedObject, PortableEmissionError> {
    match base {
        PortableDiffBase::Genesis => build_genesis_lfsd(output, network_revision, artifact, limits),
        PortableDiffBase::Artifact(base) => {
            build_artifact_lfsd(base, network_revision, artifact, limits)
        }
    }
}

pub(super) fn verify_target_relation_projection(
    output: &CompilationOutput,
    target: RegistryCheckedObjectView<'_>,
) -> Result<(), PortableEmissionError> {
    let target_index =
        ArtifactIndex::build(target, PortableEmissionError::InternalBindingMismatch)?;
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
) -> Result<OwnedObject, PortableEmissionError> {
    if base.kind() != PortableObjectKind::CanonicalArtifact {
        return Err(PortableEmissionError::InvalidDiffBaseKind);
    }
    let base_view = base.registry_view();
    let target_view = preflight_object_values_v1(
        target_artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    let base_index =
        ArtifactIndex::build(base_view, PortableEmissionError::DiffBaseSemanticMismatch)?;
    let target_index =
        ArtifactIndex::build(target_view, PortableEmissionError::InternalBindingMismatch)?;
    verify_artifact_diff_compatibility(base_view, target_view, &base_index, &target_index)?;

    let base_network_revision = network_revision(base.bytes(), limits)?;
    let base_digest = sha256(base.bytes());
    let base_length = ExactByteLength::new(
        u64::try_from(base.bytes().len()).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
    );
    let entity_changes = artifact_entity_changes(&base_index, &target_index)?;
    let relation_changes = artifact_relation_changes(&base_index, &target_index)?;
    let geometry_changes = artifact_geometry_changes(&base_index, &target_index)?;
    let static_rule_changes = artifact_static_rule_changes(&base_index, &target_index)?;
    let spatial_changes = artifact_spatial_configuration_changes(&base_index, &target_index)?;

    Ok(OwnedObject {
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
                            OwnedValue::Sha256(base_network_revision.into_digest().into_bytes()),
                        ),
                        field(4, OwnedValue::Sha256(base_digest.into_bytes())),
                        field(5, OwnedValue::U64(base_length.get())),
                        field(6, OwnedValue::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                        field(
                            7,
                            OwnedValue::Sha256(target_network_revision.into_digest().into_bytes()),
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
        ]
        .into_boxed_slice(),
    })
}
