//! Core step 输出事件。

/// v0.1 事件类型占位。
///
/// #10 只实现 fixed-step tick 与 vehicle state，不产生 route transition
/// event。后续 route following issue 会在该枚举中补充实际事件。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreEvent {}
