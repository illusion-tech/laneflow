//! 第一方道路编辑编制模型与 FlatBuffers writer。
//!
//! 本模块是编辑器和程序化生成器使用的字段私有 production authoring API。编译输入仍
//! 只接受后续 reader 切片定义的受检 size-prefixed bytes；这里不公开 generated table。

mod builder;
mod input;
#[allow(
    dead_code,
    reason = "consumed by the staged RoadEditingSource Typed AST lowering"
)]
mod location;
mod model;
#[allow(
    dead_code,
    reason = "semantic preflight is consumed by the following shared-admission slice"
)]
mod preflight;
#[allow(
    dead_code,
    reason = "verified view is consumed by the following shared-admission slice"
)]
pub(crate) mod reader;
mod rules;
mod writer;

pub use builder::{RoadEditingSourceModule, RoadEditingSourceModuleBuilder};
pub use input::{InvalidRoadEditingModuleInput, RoadEditingModuleInput};
pub use model::*;
pub use writer::{OwnedRoadEditingSourceBuffer, RoadEditingSourceWriter};
