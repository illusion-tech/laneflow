//! #1166：同次运行的表现与计时，诊断构建的数值不得进入正常延迟统计。

mod driver;
pub(crate) mod events;
mod frames;
mod matrix;
mod presentation;
mod resources;
mod runner;

pub use matrix::{matrix, prepare_oracles};
pub use runner::{guardrail, run};
