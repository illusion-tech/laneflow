//! 跨语言 writer golden fixture 验收（#376）：C++/C# writer 产出的 size-prefixed LFRE
//! bytes 必须被生产 reader（`add_road_editing_module`）无诊断接受。
//! fixture 的再生成流程与钉版来源见 `tools/lfre-crosslang-writer/README.md`。

use std::path::Path;

use laneflow_compiler::road_editing::RoadEditingModuleInput;
use laneflow_compiler::{CompilationUnitBuilder, CompileLimits};

const EXPECTED_SOURCE_DOCUMENT_KEY: &str = "roads/crosslang-writer";

fn assert_reader_accepts(fixture_name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lfre-crosslang")
        .join(fixture_name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("无法读取 fixture `{}`: {error}", path.display()));

    // framing 快检：size prefix + LFRE identifier 必须在 verifier 之前自洽。
    assert!(bytes.len() >= 12, "{fixture_name}: 长度不足 framing 下限");
    let declared = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    assert_eq!(
        declared,
        bytes.len() - 4,
        "{fixture_name}: size prefix 与实际长度不一致"
    );
    assert_eq!(&bytes[8..12], b"LFRE", "{fixture_name}: identifier 不匹配");

    let limits = CompileLimits::p100_initial_v1();
    let mut builder = CompilationUnitBuilder::new(limits);
    let input = RoadEditingModuleInput::try_new(EXPECTED_SOURCE_DOCUMENT_KEY, &bytes, None)
        .expect("fixture 的 expected key/bytes 必须满足输入契约");
    builder
        .add_road_editing_module(input)
        .unwrap_or_else(|bundle| {
            panic!("{fixture_name}: 生产 reader 必须接受跨语言 writer bytes，诊断: {bundle:?}")
        });
}

#[test]
fn cpp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("cpp_writer.lfre");
}

#[test]
fn csharp_writer_fixture_is_accepted_by_production_reader() {
    assert_reader_accepts("csharp_writer.lfre");
}
