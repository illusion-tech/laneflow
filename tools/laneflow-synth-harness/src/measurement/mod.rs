//! #1166：同次运行的表现与计时，诊断构建的数值不得进入正常延迟统计。

mod driver;
pub(crate) mod events;
mod frames;
mod load;
mod matrix;
mod presentation;
mod resources;
// 诊断对照 D 分支提为 crate 可见：准入隔离负例直接调用正式 oracle 预检。
pub(crate) mod runner;

pub use matrix::{RowSelection, matrix, prepare_oracles};
pub use runner::{RunConfig, guardrail, run};
