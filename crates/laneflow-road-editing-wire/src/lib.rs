//! `LF-ROAD-EDITING-SOURCE-v1` 的私有 FlatBuffers 生成绑定。
//!
//! 本 package 不属于 LaneFlow 产品 API，也不提供未经验证的编译入口。手写编译器只从
//! `laneflow-compiler` 的受检道路编辑入口使用这些类型，且不得重导出 generated table。

#[rustfmt::skip]
#[path = "generated/road-editing_generated.rs"]
pub mod generated;
