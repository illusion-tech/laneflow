//! 第一方道路编辑编制模型与 FlatBuffers writer。
//!
//! 本模块是编辑器和程序化生成器使用的字段私有 production authoring API。编译输入仍
//! 只接受原子 reader/admission 路径中的受检 size-prefixed bytes；这里不公开 generated
//! table 或 verifier 后的借用 view。

mod admission;
mod builder;
mod compile_geometry;
mod cut_stations;
#[allow(
    dead_code,
    reason = "consumed by the staged RoadEditingSource geometry lowering"
)]
mod geometry;
mod input;
mod location;
mod lowering;
mod model;
mod native_bounds;
mod native_input;
mod station_proof;
pub use native_bounds::{NativeChord, NativeChordBounds, NativeClearance, NativeUnprovenReason};
pub(crate) mod native_curve;
mod preflight;
mod read_curve;
/// 受检 RoadEditingSource 的原子 reader 与 admission 入口。
pub(crate) mod reader;
pub use read_curve::RoadEditingCurveOwner;
mod rules;
mod writer;

pub use builder::{RoadEditingSourceModule, RoadEditingSourceModuleBuilder};
pub use input::{InvalidRoadEditingModuleInput, RoadEditingModuleInput};
pub use model::*;
pub use writer::{OwnedRoadEditingSourceBuffer, RoadEditingSourceWriter};
