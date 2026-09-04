#![no_main]

//! Writer 合法输出 + 结构感知定点 mutation → admission 的 differential target。
//!
//! 以 `RoadEditingSourceWriter` 产出的合法 size-prefixed LFRE bytes 为种子，把
//! fuzzer 输入解释为定点 mutation 程序（截断 / 字节翻转 / offset 覆写，上界
//! `MAX_MUTATION_OPS` 次，保证单次执行有界），再送入
//! `CompilationUnitBuilder::add_road_editing_module`：
//!
//! - 任何 mutation 结果必须保持失败关闭、不 panic；
//! - 未改动的 writer 输出必须始终被接受（writer ↔ reader 一致性 oracle），
//!   违例即 panic，由 libFuzzer 记录为 crash artifact；
//! - mutation 后仍被接受时，当前公共 API 不支持把已接入模块再序列化回 LFRE
//!   bytes，roundtrip 断言按设计弱化为“接受路径不 panic 且无附带诊断”
//!   （`Ok` 类型上不携带 diagnostics）。

use std::hint::black_box;
use std::sync::OnceLock;

use laneflow_compiler::road_editing::{
    CanonicalFrameInput, RoadEditingDeclaration, RoadEditingModuleHeader, RoadEditingModuleInput,
    RoadEditingProvenance, RoadEditingSourceModuleBuilder, RoadEditingSourceWriter,
};
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile,
};
use libfuzzer_sys::fuzz_target;

const SEED_NAMESPACE: &str = "city";
const SEED_DOCUMENT_KEY: &str = "roads/fuzz-differential";
const MAX_MUTATION_OPS: usize = 16;

fn build_seed() -> Option<Vec<u8>> {
    let limits = CompileLimits::p100_initial_v1();
    let header = RoadEditingModuleHeader::try_new(
        SEED_NAMESPACE,
        SEED_DOCUMENT_KEY,
        Vec::new(),
        RoadEditingProvenance::direct("fuzz seed").ok()?,
    )
    .ok()?;
    let mut builder = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .ok()?;
    builder
        .add_declaration(RoadEditingDeclaration::CanonicalFrame(
            CanonicalFrameInput::try_new("frame").ok()?,
        ))
        .ok()?;
    let buffer = RoadEditingSourceWriter::new(&limits)
        .write(builder.finish().ok()?)
        .ok()?;
    Some(buffer.as_bytes().to_vec())
}

fn seed_bytes() -> &'static [u8] {
    static SEED: OnceLock<Vec<u8>> = OnceLock::new();
    SEED.get_or_init(|| {
        // 不变量 oracle：writer 必须能为最小合法模块产出 bytes。失败不是预期错误，
        // 而是 writer/model 回归，以 panic 上报 crash artifact。
        build_seed().unwrap_or_else(|| {
            panic!("LFRE writer invariant broken: cannot build canonical seed module")
        })
    })
}

fn check_unmutated_seed_admits(seed: &[u8]) {
    static ANCHOR: OnceLock<()> = OnceLock::new();
    ANCHOR.get_or_init(|| {
        // 不变量 oracle：writer 产出的未改动 bytes 必须被 admission 接受。
        let Ok(input) = RoadEditingModuleInput::try_new(SEED_DOCUMENT_KEY, seed, None) else {
            return;
        };
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        if let Err(bundle) = builder.add_road_editing_module(input) {
            panic!("LFRE writer/reader differential: unmutated writer output rejected: {bundle}");
        }
    });
}

fn mutate(seed: &[u8], program: &[u8]) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for chunk in program.chunks_exact(4).take(MAX_MUTATION_OPS) {
        let at = usize::from(u16::from_le_bytes([chunk[1], chunk[2]]));
        match chunk[0] % 3 {
            // 截断到 [0, len] 内的任意长度。
            0 => bytes.truncate(at % (bytes.len() + 1)),
            // 翻转选中字节。
            1 if !bytes.is_empty() => {
                let index = at % bytes.len();
                bytes[index] ^= chunk[3];
            }
            // offset 覆写选中字节。
            2 if !bytes.is_empty() => {
                let index = at % bytes.len();
                bytes[index] = chunk[3];
            }
            _ => {}
        }
    }
    bytes
}

fuzz_target!(|data: &[u8]| {
    let seed = seed_bytes();
    check_unmutated_seed_admits(seed);
    let mutated = mutate(seed, data);
    let Ok(input) = RoadEditingModuleInput::try_new(SEED_DOCUMENT_KEY, &mutated, None) else {
        return;
    };
    let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
    match builder.add_road_editing_module(input) {
        Ok(_) => {
            // 接受路径：弱化 roundtrip 断言——不 panic 即通过；builder 析构
            // （含已接入模块的 Typed AST 释放）仍被本分支覆盖。
        }
        Err(bundle) => {
            for diagnostic in bundle.diagnostics() {
                black_box(diagnostic.code());
                black_box(diagnostic.payload());
                black_box(diagnostic.primary_location());
                black_box(diagnostic.stable_key());
            }
            black_box(bundle.has_errors());
        }
    }
});
