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
        /// 策略集的稳定身份。
        id: RightOfWayPolicySetId,
        /// 策略集在同次规范编译中的序号。
        ordinal: RightOfWayPolicySetOrdinal,
    },
    /// 策略局部具名成员；localIndex 在发射和检查时分别按 key 重建。
    Member {
        /// 拥有该成员的策略集稳定身份。
        owner: RightOfWayPolicySetId,
        /// 策略局部成员种类。
        kind: PolicyLocalMemberKind,
        /// owner-local 成员键；不是来源物理下标或独立 StableId。
        key: Box<str>,
    },
    /// Movement 显式方向字段。主来源仍取 Movement 声明。
    MovementDirection {
        /// 目标 Movement 的稳定身份。
        id: MovementId,
    },
}

/// 只能由编译器内部从正式来源和同次身份映射组成，不接受 LFSM 回填。
pub(crate) struct PolicySourceInput<'a> {
    pub target: PolicySourceTarget,
    pub owner_module: u32,
    pub primary: &'a SourceLocation,
    pub contributing: &'a [SourceLocation],
}

/// 已冻结的路权来源记录：语义目标加主/伴随来源位置。
pub(super) struct PolicySourceRecord {
    target: PolicySourceTarget,
    primary: SourceLocationRecord,
    contributing: crate::ledger::Frozen<SourceLocationRecord>,
}

impl PolicySourceRecord {
    /// 返回点转交宿主：伴随来源表的授予随记录转交。
    pub(super) fn transfer_to_host(&mut self) {
        self.contributing.transfer_to_host();
    }
}

/// 已经过共同文档所有权检查的只读路权来源。
#[derive(Clone, Copy)]
pub struct PolicySourceView<'a> {
    pub(super) source_map: &'a ValidatedSourceMapInput,
    pub(super) record: &'a PolicySourceRecord,
}

impl<'a> PolicySourceView<'a> {
    /// 返回该记录绑定的语义目标。
    #[must_use]
    pub const fn target(&self) -> &'a PolicySourceTarget {
        &self.record.target
    }

    /// 返回规范主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'a> {
        self.source_map.location(&self.record.primary)
    }

    /// C(view) 由消费方对这些真实位置按完整语义值排序去重。
    pub fn contributing_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = SourceLocationView<'a>> + use<'a> {
        let source_map = self.source_map;
        self.record
            .contributing
            .iter()
            .map(move |r| source_map.location(r))
    }
}

/// 冻结策略来源记录，全部成功后返回；不修改旧记录。
///
/// 记录表与各记录的伴随来源表由 `pass` 逐项授予；成员键的副本由调用方集中授予。
pub(super) fn freeze_policy_sources(
    unit: &CompilationUnit,
    inputs: &[PolicySourceInput<'_>],
    pass: &crate::ledger::PassBudget<'_>,
) -> Result<crate::ledger::Frozen<PolicySourceRecord>, DiagnosticBundle> {
    let occurrences = inputs.iter().fold(0_u64, |total, input| {
        total.saturating_add(1 + input.contributing.len() as u64)
    });
    if let Some(violation) = unit.limits.source_limits().exceeded(
        crate::SourceProcessingDimension::RelationOccurrenceCount,
        occurrences,
    ) {
        let crate::limits::ResourceLimitExceeded {
            dimension,
            limit,
            observed,
        } = violation;
        return Err(DiagnosticBundle::single(
            Diagnostic::compile_limit_exceeded_at(
                dimension,
                limit,
                observed,
                inputs.first().map(|i| i.primary.clone()),
                None,
            ),
        ));
    }
    let mut records = pass.vec(inputs.len())?;
    for input in inputs {
        // 输入来自同次受检模块；不得用另一模块的登记文档或未登记文档伪造位置。
        let primary = unit
            .resolve_source_location_for_module(input.owner_module, input.primary)?
            .into();
        let mut contributing = pass.vec(input.contributing.len())?;
        for location in input.contributing {
            contributing.push(
                unit.resolve_source_location_for_module(input.owner_module, location)?
                    .into(),
            )?;
        }
        records.push(PolicySourceRecord {
            target: input.target.clone(),
            primary,
            contributing: contributing.freeze()?,
        })?;
    }
    records.sort_unstable_by(|a, b| a.target.cmp(&b.target));
    Ok(records.freeze()?)
}
