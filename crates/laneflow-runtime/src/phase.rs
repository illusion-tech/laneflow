//! 串行阶段的借用能力；准备计算不取得 `&mut TrafficWorld`。

use core::ops::Deref;

use crate::TrafficWorld;
use crate::conflict::{ConflictRead, ConflictResolution};
use crate::migration_journal::MigrationDeltaJournal;
use crate::state::{CommittedWorldState, DerivedIndexes, TickWorkspace, WorldBindingState};

/// 同一拍初基线的只读投影，不含工作区或管理操作。
#[derive(Clone, Copy)]
pub(crate) struct StepReadView<'a> {
    pub(crate) binding: &'a WorldBindingState,
    pub(crate) committed: &'a CommittedWorldState,
    pub(crate) derived: &'a DerivedIndexes,
}

impl<'a> StepReadView<'a> {
    pub(crate) fn conflict_read(self) -> ConflictRead<'a> {
        ConflictRead::committed(&self.committed.conflict, &self.derived.conflict)
    }
}

/// 只读业务状态；唯一的可变操作是 Conflict 容器的受限容量准备。
pub(crate) struct StepCommitted<'a>(&'a mut CommittedWorldState);

impl Deref for StepCommitted<'_> {
    type Target = CommittedWorldState;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// 构建/失效权限由专门接口授予，不提供任意索引写入。
pub(crate) struct StepDerived<'a>(&'a mut DerivedIndexes);

impl Deref for StepDerived<'_> {
    type Target = DerivedIndexes;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl StepCommitted<'_> {
    pub(crate) fn prepare_conflict<'a>(
        &'a mut self,
        derived: &'a mut StepDerived<'_>,
        workspace: &'a mut crate::conflict::ConflictWorkspace,
    ) -> ConflictResolution<'a> {
        ConflictResolution::new(&mut self.0.conflict, &mut derived.0.conflict, workspace)
    }
}

/// P2～P6 只允许产生暂存结果；资源提交方法不在此视图中。
pub(crate) struct StepWorkspace<'a> {
    pub(crate) binding: &'a WorldBindingState,
    pub(crate) committed: StepCommitted<'a>,
    pub(crate) derived: StepDerived<'a>,
    pub(crate) workspace: &'a mut TickWorkspace,
    pub(crate) journal_armed: bool,
}

impl StepWorkspace<'_> {
    pub(crate) fn read_view(&self) -> StepReadView<'_> {
        StepReadView {
            binding: self.binding,
            committed: &self.committed,
            derived: &self.derived,
        }
    }

    pub(crate) fn conflict_read(&self) -> ConflictRead<'_> {
        ConflictRead::new(
            &self.committed.conflict,
            &self.derived.conflict,
            &self.workspace.conflict,
        )
    }
}

/// P7 或两次 step 之间的既有生命周期/恢复边界使用此写视图。
pub(crate) struct CommittedStateMut<'a> {
    pub(crate) binding: &'a WorldBindingState,
    pub(crate) committed: &'a mut CommittedWorldState,
    pub(crate) derived: &'a mut DerivedIndexes,
    pub(crate) workspace: &'a mut TickWorkspace,
    pub(crate) journal: &'a mut Option<MigrationDeltaJournal>,
}

impl CommittedStateMut<'_> {
    pub(crate) fn read_view(&self) -> StepReadView<'_> {
        StepReadView {
            binding: self.binding,
            committed: self.committed,
            derived: self.derived,
        }
    }
    pub(crate) fn conflict_read(&self) -> ConflictRead<'_> {
        ConflictRead::new(
            &self.committed.conflict,
            &self.derived.conflict,
            &self.workspace.conflict,
        )
    }
}

impl TrafficWorld {
    pub(crate) const fn read_view(&self) -> StepReadView<'_> {
        StepReadView {
            binding: &self.binding,
            committed: &self.committed,
            derived: &self.derived,
        }
    }

    pub(crate) fn step_workspace(&mut self) -> StepWorkspace<'_> {
        StepWorkspace {
            binding: &self.binding,
            committed: StepCommitted(&mut self.committed),
            derived: StepDerived(&mut self.derived),
            workspace: &mut self.workspace,
            journal_armed: self.admin.migration_journal.is_some(),
        }
    }

    pub(crate) fn committed_mut(&mut self) -> CommittedStateMut<'_> {
        CommittedStateMut {
            binding: &self.binding,
            committed: &mut self.committed,
            derived: &mut self.derived,
            workspace: &mut self.workspace,
            journal: &mut self.admin.migration_journal,
        }
    }
}
