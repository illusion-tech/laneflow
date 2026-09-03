//! W1 的受检来源投影接缝；正式前端在 W2 从同一 CompilationUnit 提供输入。

use super::*;
use laneflow_static_contract::{
    PolicyLocalMemberKind, RightOfWayPolicySetId, RightOfWayPolicySetOrdinal,
};

/// 路权来源绑定的语义目标。成员 key 不是来源物理下标或独立 StableId。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PolicySourceTarget {
    /// 策略声明，序号来自同次规范编译。
    Declaration {
        id: RightOfWayPolicySetId,
        ordinal: RightOfWayPolicySetOrdinal,
    },
    /// 策略局部具名成员；localIndex 在发射和检查时分别按 key 重建。
    Member {
        owner: RightOfWayPolicySetId,
        kind: PolicyLocalMemberKind,
        key: Box<str>,
    },
    /// Movement 显式方向字段。主来源仍取 Movement 声明。
    MovementDirection { id: MovementId },
}

/// 只能由编译器内部从正式来源和同次身份映射组成，不接受 LFSM 回填。
pub(crate) struct PolicySourceInput<'a> {
    pub target: PolicySourceTarget,
    pub owner_module: u32,
    pub primary: &'a SourceLocation,
    pub contributing: &'a [SourceLocation],
}

pub(super) struct PolicySourceRecord {
    target: PolicySourceTarget,
    primary: SourceLocationRecord,
    contributing: Box<[SourceLocationRecord]>,
}

/// 已经过共同文档所有权检查的只读路权来源。
#[derive(Clone, Copy)]
pub struct PolicySourceView<'a> {
    pub(super) source_map: &'a ValidatedSourceMapInput,
    pub(super) record: &'a PolicySourceRecord,
}

impl PolicySourceView<'_> {
    #[must_use]
    pub const fn target(&self) -> &PolicySourceTarget {
        &self.record.target
    }

    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(&self.record.primary)
    }

    /// C(view) 由消费方对这些真实位置按完整语义值排序去重。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        self.record
            .contributing
            .iter()
            .map(|r| self.source_map.location(r))
    }
}

/// 与现有源映射同时计量，全部成功后返回；不修改旧记录。
pub(super) fn freeze_policy_sources(
    unit: &CompilationUnit,
    inputs: &[PolicySourceInput<'_>],
    prior_live: u64,
    prior_output: u64,
    prior_scratch: u64,
) -> Result<(Box<[PolicySourceRecord]>, u64, u64), DiagnosticBundle> {
    let mut owned = (inputs.len() as u64).saturating_mul(size_of::<PolicySourceRecord>() as u64);
    let mut logical = 0_u64;
    let mut occurrences = 0_u64;
    for input in inputs {
        owned = owned.saturating_add(
            (input.contributing.len() as u64)
                .saturating_mul(size_of::<SourceLocationRecord>() as u64),
        );
        logical = logical.saturating_add(32);
        occurrences = occurrences.saturating_add(1 + input.contributing.len() as u64);
        if let PolicySourceTarget::Member { key, .. } = &input.target {
            owned = owned.saturating_add(key.len() as u64);
            logical = logical.saturating_add(key.len() as u64 + 4);
        }
        for location in core::iter::once(input.primary).chain(input.contributing) {
            // 只借用进行解析前计量；共享 context 已归共同准入的 unit 所有。
            logical = logical.saturating_add(match location {
                SourceLocation::Text(_) => TEXT_SOURCE_LOCATION_LOGICAL_BYTES,
                SourceLocation::RoadEditing(_) => ROAD_EDITING_LOCATION_PAYLOAD_LOGICAL_BYTES,
            });
        }
    }
    let observations = [
        (CompileLimitDimension::RelationOccurrenceCount, occurrences),
        (
            CompileLimitDimension::StageScratchBytes,
            prior_scratch.saturating_add(owned),
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            prior_live.saturating_add(owned),
        ),
        (
            CompileLimitDimension::OutputBytes,
            prior_output.saturating_add(logical),
        ),
    ];
    for (dimension, actual) in observations {
        let limit = unit.limits.value(dimension);
        if actual > limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded_at(
                    dimension,
                    limit,
                    actual,
                    inputs.first().map(|i| i.primary.clone()),
                    None,
                ),
            ));
        }
    }
    let mut records = Vec::with_capacity(inputs.len());
    for input in inputs {
        // 输入来自同次受检模块；不得用另一模块的登记文档或未登记文档伪造位置。
        let primary = unit
            .resolve_source_location_for_module(input.owner_module, input.primary)?
            .into();
        let contributing = input
            .contributing
            .iter()
            .map(|location| {
                unit.resolve_source_location_for_module(input.owner_module, location)
                    .map(Into::into)
            })
            .collect::<Result<Box<[_]>, _>>()?;
        records.push(PolicySourceRecord {
            target: input.target.clone(),
            primary,
            contributing,
        });
    }
    records.sort_unstable_by(|a, b| a.target.cmp(&b.target));
    Ok((records.into_boxed_slice(), owned, logical))
}
