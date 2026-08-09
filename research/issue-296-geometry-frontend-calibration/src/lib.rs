//! #296 Geometry 文档前端 §9 性能校准的非生产 harness。
//!
//! 本 crate 只在 `research/` 非生产边界内生成校准 workload fixture、workload manifest、
//! cross-record 验证与 release 测量证据；它不进入生产编译器公共 API 或依赖图，也不
//! 改变任何已冻结工作负载语义。校验顺序复用 #308 的 trusted contract → schema/manifest
//! exact bytes → evidence cross-record validation。

pub mod container;
pub mod corridor;
pub mod counts;
pub mod emit;
pub mod manifest;
pub mod selfcheck;
pub mod twin;
pub mod validator;
