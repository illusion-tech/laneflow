#![no_main]

//! 任意字节 → `CompilationUnitBuilder::add_road_editing_module`。
//!
//! 覆盖 framing 检查、FlatBuffers verifier（depth/table/apparent-size 限制）、
//! preflight 与 lowering 的失败关闭行为：任意字节下入口不得 panic、unwind 或
//! 死循环；拒绝路径返回的 `DiagnosticBundle` 被逐项消费，以覆盖错误路径的
//! 构造与析构。

use std::hint::black_box;

use laneflow_compiler::road_editing::RoadEditingModuleInput;
use laneflow_compiler::{CompilationUnitBuilder, CompileLimits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // expected key 固定且合法，`try_new` 只在 key 非法时失败，此处恒为 Ok。
    let Ok(input) = RoadEditingModuleInput::try_new("roads/fuzz", data, None) else {
        return;
    };
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    // 成功或失败关闭均可；唯一硬性要求是 admission 原子失败、不 panic。
    if let Err(bundle) = builder.add_road_editing_module(input) {
        for diagnostic in bundle.diagnostics() {
            black_box(diagnostic.code());
            black_box(diagnostic.severity());
            black_box(diagnostic.payload());
            black_box(diagnostic.primary_location());
            black_box(diagnostic.related_locations());
            black_box(diagnostic.stable_key());
        }
        black_box(bundle.has_errors());
        black_box(bundle.diagnostics_truncated());
    }
});
