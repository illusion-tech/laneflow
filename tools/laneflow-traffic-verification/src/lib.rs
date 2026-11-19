//! 多个验证执行器（harness）共享的交通验证判据。
//!
//! 本 crate 只承载独立参考判据与纯事实类型：它们消费公开、可解释的拍前/拍后
//! 事实，不调用也不读取生产 `TrafficWorld` 的门决策、困境区豁免或有效许可
//! 结果；生产 Runtime、Bevy 与两类生成器不得反向依赖本包。规约组合、验证
//! 计划/会话及证据准入由各 harness 的受检构造入口组织，不在此建立包含全部
//! workload 的全局枚举。
//!
//! 判据语义以 `docs/design/traffic-verification.md`（已接受，#1458 合入）为
//! 正本；信号许可参考沿用 `lf-synth-execution-2` 的独立判据（#1456）。

pub mod body;
pub mod signal;
