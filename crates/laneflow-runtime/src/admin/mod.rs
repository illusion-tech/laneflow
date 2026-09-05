//! 管理平面：快照、恢复、路网切换及迁移日志。

pub(crate) mod cutover;
pub(crate) mod cutover_migration;
pub(crate) mod cutover_transaction;
pub(crate) mod format_admission;
pub(crate) mod migration_journal;
pub(crate) mod snapshot;
pub(crate) mod snapshot_digest;
pub(crate) mod snapshot_restore;
pub(crate) mod state;
