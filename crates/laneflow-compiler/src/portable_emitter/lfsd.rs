/// 实体索引、受检行字段读取与跨修订差异兼容核对等 LFSD 构建共用基础。
pub(super) mod base;
mod changes;
mod entity;
mod geometry;
mod policy;
/// 路权策略局部成员变更的配对投影。
pub(super) mod policy_change;
mod policy_check;
pub(super) use policy_check::check_policy_diff;
pub use policy_check::check_portable_policy_diff;
mod relation;

use base::{ArtifactIndex, verify_artifact_diff_compatibility};
use changes::{ChangeBounds, DiffHeader, SpatialChange, emit_lfsd};
use entity::{
    artifact_entity_changes, artifact_static_rule_changes, field_change_bounds,
    genesis_entity_bound, genesis_entity_changes,
};
use geometry::{
    artifact_geometry_changes, artifact_spatial_configuration_change, genesis_geometry_changes,
    geometry_change_bounds,
};
use policy::{policy_reference_plan, validate_policy_references};
use relation::{
    RelationChange, artifact_relation_changes, artifact_relation_count, artifact_relation_plan,
    artifact_relation_tuples, genesis_relation_changes,
};

use super::budget::Accounting;
use super::relations::{
    canonical_relation_count, canonical_relation_scratch_len, canonical_relation_tuples,
    entity_stable_id,
};
use super::tree::{ObjectTree, build_tree};
use super::*;

fn build_genesis_lfsd(
    output: &CompilationOutput,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    accounting: Accounting<'_>,
) -> Result<ObjectTree, PortableEmissionError> {
    let target = preflight_object_values(
        artifact.bytes(),
        PortableObjectKind::CanonicalArtifact,
        limits,
    )?
    .registry_view();
    // 目标实体索引作为索引编译遍的输出，供变化编译遍使用。
    let index_pass = accounting.admit("genesis-diff-index", ArtifactIndex::plan(target))?;
    let target_index = ArtifactIndex::build(
        target,
        PortableEmissionError::InternalBindingMismatch,
        &index_pass,
    )?;
    index_pass.finish(target_index.granted_bytes())?;

    let lir = output.lir().unit();
    let mut bounds = ChangeBounds {
        entity: genesis_entity_bound(target)?,
        relation: canonical_relation_count(lir),
        ..ChangeBounds::default()
    };
    // Genesis 不建立几何值表，只取目标一侧的变化与暂存上界。
    geometry_change_bounds(None, target, &mut bounds)?;
    let members = policy_change::policy_change_bounds(None, &target_index, &mut bounds)?;
    let pass = accounting.admit(
        "genesis-diff-changes",
        bounds
            .plan()
            .plus(members)
            .array::<u32>(canonical_relation_scratch_len(lir)),
    )?;
    let mut changes = bounds.grant(&pass)?;
    policy_change::policy_changes(
        None,
        &target_index,
        &pass,
        &mut changes.policy,
        &mut changes.staged,
    )?;
    genesis_entity_changes(target, &mut changes.entity)?;
    genesis_relation_changes(lir, &pass, &mut changes.relation)?;
    genesis_geometry_changes(lir, target, &mut changes.geometry, &mut changes.staged)?;
    changes.spatial = Some(SpatialChange::Genesis(
        target
            .section(4)
            .and_then(|section| section.table(0))
            .and_then(|table| table.row(0))
            .ok_or(PortableEmissionError::InternalBindingMismatch)?
            .bytes(),
    ));
    pass.finish(changes.granted_bytes())?;
    drop(target_index);

    let header = DiffHeader::Genesis {
        network_revision,
        artifact_digest: artifact.digest(),
        artifact_length: artifact.byte_length().get(),
    };
    build_tree(
        accounting,
        "lfsd-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_lfsd(t, &header, &changes)),
    )
}

/// 按显式 base 选择构建 LFSD 语义差异对象树。
///
/// `Genesis` 输出相对空基线的全量新增差异；`Artifact` 输出相对基线制品的增量差异，
/// 并返回调用方必须核对的期望 base 绑定。实体索引、变化表与对象树依次在三个编译遍内
/// 由发射账本授予：索引与变化表分别作为前一遍的输出延续到后一遍结束。
pub(super) fn build_lfsd(
    output: &CompilationOutput,
    base: PortableDiffBase<'_>,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    accounting: Accounting<'_>,
) -> Result<(ObjectTree, ExpectedSemanticDiffBase), PortableEmissionError> {
    match base {
        PortableDiffBase::Genesis => Ok((
            build_genesis_lfsd(output, network_revision, artifact, limits, accounting)?,
            ExpectedSemanticDiffBase::Genesis,
        )),
        PortableDiffBase::Artifact(base) => {
            build_artifact_lfsd(base, network_revision, artifact, limits, accounting)
        }
    }
}

/// 核对目标 LFCA 投影出的关系元组与 LIR 规范关系完全一致。
///
/// 先在索引编译遍内重建目标制品索引并校验路权策略引用，索引作为输出延续；再在比较
/// 编译遍内按索引计数授予两侧关系元组。不一致即报内部绑定错误，返回前全部释放。
pub(super) fn verify_target_relation_projection(
    output: &CompilationOutput,
    target: RegistryCheckedObjectView<'_>,
    accounting: Accounting<'_>,
) -> Result<(), PortableEmissionError> {
    let index_pass = accounting.admit(
        "relation-projection-index",
        ArtifactIndex::plan(target).plus(policy_reference_plan(target)),
    )?;
    let target_index = ArtifactIndex::build(
        target,
        PortableEmissionError::InternalBindingMismatch,
        &index_pass,
    )?;
    validate_policy_references(
        &target_index,
        &index_pass,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    index_pass.finish(target_index.granted_bytes())?;

    let lir = output.lir().unit();
    let pass = accounting.admit(
        "relation-projection",
        artifact_relation_plan(
            &target_index,
            PortableEmissionError::InternalBindingMismatch,
        )?
        .array::<RelationTuple>(canonical_relation_count(lir))
        .array::<u32>(canonical_relation_scratch_len(lir)),
    )?;
    let projected = artifact_relation_tuples(
        &target_index,
        PortableEmissionError::InternalBindingMismatch,
        &pass,
    )?;
    let canonical = canonical_relation_tuples(lir, &pass)?;
    if projected[..] != canonical[..] {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    drop((projected, canonical));
    pass.finish(0)?;
    drop(target_index);
    Ok(())
}

fn build_artifact_lfsd(
    base: ValueCheckedObjectView<'_>,
    target_network_revision: NetworkRevisionId,
    target_artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    accounting: Accounting<'_>,
) -> Result<(ObjectTree, ExpectedSemanticDiffBase), PortableEmissionError> {
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
    // 两侧索引同时存续，作为索引编译遍的输出；两次策略引用核对先后进行，键在各自结束时
    // 释放，取峰值。
    let index_pass = accounting.admit(
        "artifact-diff-index",
        ArtifactIndex::plan(base_view)
            .plus(ArtifactIndex::plan(target_view))
            .plus(policy_reference_plan(base_view).max(policy_reference_plan(target_view))),
    )?;
    let base_index = ArtifactIndex::build(
        base_view,
        PortableEmissionError::DiffBaseSemanticMismatch,
        &index_pass,
    )?;
    let target_index = ArtifactIndex::build(
        target_view,
        PortableEmissionError::InternalBindingMismatch,
        &index_pass,
    )?;
    validate_policy_references(
        &base_index,
        &index_pass,
        PortableEmissionError::DiffBaseSemanticMismatch,
    )?;
    validate_policy_references(
        &target_index,
        &index_pass,
        PortableEmissionError::InternalBindingMismatch,
    )?;
    verify_artifact_diff_compatibility(base_view, target_view, &base_index, &target_index)?;
    index_pass.finish(base_index.granted_bytes() + target_index.granted_bytes())?;

    let base_network_revision = network_revision_from_checked(base)?;
    let base_digest = sha256(base.bytes());
    let base_length = ExactByteLength::new(
        u64::try_from(base.bytes().len()).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
    );

    // 变化表：容量按两侧行数求上界。关系元组表、几何值表与策略成员表先后使用，取峰值。
    let mut bounds = ChangeBounds::default();
    field_change_bounds(&base_index, &target_index, &mut bounds)?;
    bounds.relation =
        artifact_relation_count(&base_index, PortableEmissionError::DiffBaseSemanticMismatch)?
            .checked_add(artifact_relation_count(
                &target_index,
                PortableEmissionError::InternalBindingMismatch,
            )?)
            .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let tuples =
        artifact_relation_plan(&base_index, PortableEmissionError::DiffBaseSemanticMismatch)?.plus(
            artifact_relation_plan(
                &target_index,
                PortableEmissionError::InternalBindingMismatch,
            )?,
        );
    let geometries = geometry_change_bounds(Some(base_view), target_view, &mut bounds)?;
    let members =
        policy_change::policy_change_bounds(Some(&base_index), &target_index, &mut bounds)?;
    let pass = accounting.admit(
        "artifact-diff-changes",
        bounds.plan().plus(tuples.max(geometries).max(members)),
    )?;
    let mut changes = bounds.grant(&pass)?;
    artifact_entity_changes(
        &base_index,
        &target_index,
        &mut changes.entity,
        &mut changes.staged,
    )?;
    {
        let mut base_tuples = artifact_relation_tuples(
            &base_index,
            PortableEmissionError::DiffBaseSemanticMismatch,
            &pass,
        )?;
        let mut target_tuples = artifact_relation_tuples(
            &target_index,
            PortableEmissionError::InternalBindingMismatch,
            &pass,
        )?;
        artifact_relation_changes(&mut base_tuples, &mut target_tuples, &mut changes.relation)?;
    }
    artifact_geometry_changes(
        &base_index,
        &target_index,
        &pass,
        &mut changes.geometry,
        &mut changes.staged,
    )?;
    artifact_static_rule_changes(
        &base_index,
        &target_index,
        &mut changes.static_rule,
        &mut changes.staged,
    )?;
    changes.spatial = artifact_spatial_configuration_change(&base_index, &target_index)?;
    policy_change::policy_changes(
        Some(&base_index),
        &target_index,
        &pass,
        &mut changes.policy,
        &mut changes.staged,
    )?;
    pass.finish(changes.granted_bytes())?;
    drop((base_index, target_index));

    let expected_base = ExpectedSemanticDiffBase::Artifact {
        network_revision_derivation_version: NETWORK_REVISION_DERIVATION_VERSION,
        network_revision: base_network_revision,
        digest: base_digest,
        byte_length: base_length,
    };
    let header = DiffHeader::Artifact {
        base_network_revision,
        base_digest,
        base_length: base_length.get(),
        network_revision: target_network_revision,
        artifact_digest: target_artifact.digest(),
        artifact_length: target_artifact.byte_length().get(),
    };
    let tree = build_tree(
        accounting,
        "lfsd-tree",
        PortableObjectKind::SemanticDiff,
        |t| Ok::<_, PortableEmissionError>(emit_lfsd(t, &header, &changes)),
    )?;
    Ok((tree, expected_base))
}

/// 测试用：构造 Genesis LFSD 并还原为拥有型嵌套对象。
#[cfg(test)]
fn build_genesis_lfsd_owned(
    output: &CompilationOutput,
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    accounting: Accounting<'_>,
) -> Result<OwnedObject, PortableEmissionError> {
    Ok(
        build_genesis_lfsd(output, network_revision, artifact, limits, accounting)?
            .to_owned_object(),
    )
}

/// 测试用：构造制品差异 LFSD 并还原为拥有型嵌套对象。
#[cfg(test)]
fn build_artifact_lfsd_owned(
    base: ValueCheckedObjectView<'_>,
    target_network_revision: NetworkRevisionId,
    target_artifact: &PortableObjectCandidate,
    limits: FormatLimits,
    accounting: Accounting<'_>,
) -> Result<(OwnedObject, ExpectedSemanticDiffBase), PortableEmissionError> {
    let (tree, expected) = build_artifact_lfsd(
        base,
        target_network_revision,
        target_artifact,
        limits,
        accounting,
    )?;
    Ok((tree.to_owned_object(), expected))
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
                let range = duplicate_stable_id
                    .subslice_range(value)
                    .expect("stable id belongs to object bytes");
                range.start..range.end
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
            build_artifact_lfsd_owned(
                base,
                NetworkRevisionId::from_digest(Sha256Digest::ZERO),
                &target,
                limits,
                crate::portable_emitter::budget::test_accounting(u64::MAX),
            ),
            Err(PortableEmissionError::Format(FormatError::LimitExceeded {
                dimension: LimitDimension::ObjectBytes,
                actual: duplicate_stable_id.len() as u64,
                limit: duplicate_stable_id.len() as u64 - 1,
            }))
        );
    }

    #[test]
    fn change_tables_are_admitted_before_allocation_after_the_index_pass() {
        let base = preflight_object_values(
            EXPECTED_LFCA,
            PortableObjectKind::CanonicalArtifact,
            FormatLimits::HARD,
        )
        .unwrap();
        let view = base.registry_view();
        let target = close_object(EXPECTED_LFCA.to_vec().into_boxed_slice());
        // 预算恰好容纳两侧实体索引与策略引用键：索引编译遍通过，变化表计划被拒绝。
        let index_plan = ArtifactIndex::plan(view)
            .plus(ArtifactIndex::plan(view))
            .plus(policy_reference_plan(view))
            .bytes()
            .unwrap();
        let accounting = crate::portable_emitter::budget::test_accounting(index_plan);
        let error = build_artifact_lfsd(
            base,
            NetworkRevisionId::from_digest(Sha256Digest::ZERO),
            &target,
            FormatLimits::HARD,
            accounting,
        )
        .err()
        .unwrap();
        assert!(
            matches!(
                error,
                PortableEmissionError::CompileLimitExceeded {
                    dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                    limit,
                    ..
                } if limit == index_plan
            ),
            "{error:?}"
        );
        assert_eq!(accounting.ledger.live(), 0);
        assert_eq!(accounting.ledger.reserved(), 0);

        // 预算充足时变化表与对象树全部释放，账本结束时归零。
        let accounting = crate::portable_emitter::budget::test_accounting(u64::MAX);
        let (tree, _) = build_artifact_lfsd(
            base,
            NetworkRevisionId::from_digest(Sha256Digest::ZERO),
            &target,
            FormatLimits::HARD,
            accounting,
        )
        .unwrap();
        assert_eq!(accounting.ledger.live(), tree.granted_bytes());
        drop(tree);
        assert_eq!(accounting.ledger.live(), 0);
        assert_eq!(accounting.ledger.reserved(), 0);
    }
}
