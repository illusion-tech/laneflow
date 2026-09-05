//! 每世界管理数据的唯一所有者。

use super::migration_journal::MigrationDeltaJournal;

/// 切换日志及配对世代；不授予步进任意管理权限。
pub(crate) struct AdministrativeState {
    /// 武装中的迁移增量日志（#513 切片 C）：`Some` ⟺ 本世界存在在途切换事务。
    /// 武装与解除都只发生在切换事务的原子边界；溢出粘性置位，从不影响本世界
    /// 自身的提交路径。
    pub(crate) migration_journal: Option<MigrationDeltaJournal>,
    /// 日志武装轮次：每次成功武装递增（进程内守卫，不落盘）。事务绑定
    /// 武装时的轮次，配对校验一并比对——世界级恢复后重新武装的新日志
    /// 对旧事务按配对失配失败关闭，防止旧事务认领后继日志。
    pub(crate) migration_epoch: u64,
}

#[cfg(test)]
impl AdministrativeState {
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        let Self {
            migration_journal,
            migration_epoch: _,
        } = self;
        migration_journal
            .as_ref()
            .map_or(0, MigrationDeltaJournal::retained_logical_bytes)
    }
}
