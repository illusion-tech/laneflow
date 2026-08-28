//! `LFRS`（Runtime Snapshot v1）的私有 FlatBuffers 生成绑定。
//!
//! 本 package 不属于 LaneFlow 产品 API，也不提供未经验证的读取入口。手写运行时只从
//! `laneflow-runtime` 的受检快照入口使用这些类型，且不得重导出 generated table。

#[rustfmt::skip]
#[allow(
    clippy::derivable_impls,
    clippy::extra_unused_lifetimes,
    clippy::missing_safety_doc,
    reason = "fixed flatc 25.12.19 generated code; clean regeneration is the authority"
)]
#[path = "generated/runtime-snapshot_generated.rs"]
pub mod generated;

/// 只供 `laneflow-runtime` 的私有 writer/reader 实现使用；不是产品 API。
#[doc(hidden)]
pub use flatbuffers as runtime;
