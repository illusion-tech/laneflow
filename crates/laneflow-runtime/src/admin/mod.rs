//! 管理平面：快照、恢复、路网切换及迁移日志。

pub(crate) mod cutover;
/// 跨修订直移核心（#302 切换合同 §3；#513 切片 C-2）。
pub(crate) mod cutover_migration;
/// 切换事务对象：Prepare → Delta Catch-up → Quiescent Commit / 放弃。
pub(crate) mod cutover_transaction;
/// Runtime 的唯一原始格式入口：LFSD 认证及 LFRS verifier、lowering 与编码。
pub(crate) mod format_admission;
/// 迁移增量日志：已提交变更流的有界物化。
pub(crate) mod migration_journal;
/// 运行时快照的保存路径。
pub(crate) mod snapshot;
/// 与 `LFRS` 容器编码无关的 Runtime 逻辑状态摘要。
pub(crate) mod snapshot_digest;
/// 快照恢复的公开上限、错误与完整新世界结果。
pub(crate) mod snapshot_restore;
/// 每世界管理数据的唯一所有者。
pub(crate) mod state;
