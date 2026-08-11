//! 第一方道路编辑编制模型与 FlatBuffers writer。
//!
//! 本模块是编辑器和程序化生成器使用的字段私有 production authoring API。编译输入仍
//! 只接受原子 reader/admission 路径中的受检 size-prefixed bytes；这里不公开 generated
//! table 或 verifier 后的借用 view。

mod admission;
mod builder;
mod compile_geometry;
#[cfg(feature = "road-editing-g3-evidence")]
mod evidence;
#[allow(
    dead_code,
    reason = "consumed by the staged RoadEditingSource geometry lowering"
)]
mod geometry;
#[cfg(test)]
mod geometry_known_vectors;
mod input;
mod location;
mod lowering;
mod model;
mod preflight;
pub(crate) mod reader;
mod rules;
mod writer;

pub use builder::{RoadEditingSourceModule, RoadEditingSourceModuleBuilder};
#[cfg(feature = "road-editing-g3-evidence")]
pub use evidence::RoadEditingAdmissionStageDurations;
pub use input::{InvalidRoadEditingModuleInput, RoadEditingModuleInput};
pub use model::*;
pub use writer::{OwnedRoadEditingSourceBuffer, RoadEditingSourceWriter};
