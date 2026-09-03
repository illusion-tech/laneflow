//! `LF-ROAD-EDITING-SOURCE-v4` 的私有 FlatBuffers 生成绑定。
//!
//! 本 package 不属于 LaneFlow 产品 API，也不提供未经验证的编译入口。手写编译器只从
//! `laneflow-compiler` 的受检道路编辑入口使用这些类型，且不得重导出 generated table。

#[rustfmt::skip]
#[allow(
    clippy::derivable_impls,
    clippy::extra_unused_lifetimes,
    clippy::missing_safety_doc,
    reason = "fixed flatc 25.12.19 generated code; clean regeneration is the authority"
)]
#[path = "generated/road-editing_generated.rs"]
pub mod generated;

/// 只供 `laneflow-compiler` 的私有 writer/reader 实现使用；不是产品 API。
#[doc(hidden)]
pub use flatbuffers as runtime;
