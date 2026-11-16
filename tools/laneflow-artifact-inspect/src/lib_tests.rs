//! 断言辅助单元测试：相等快速路径与失配有界诊断。

use crate::assert_portable_object_matches;

#[test]
fn equal_fast_path_does_not_decode_arbitrary_bytes() {
    // 相等即通过：即使字节不是任何合法对象（未解码、未构造诊断、无探针输出）。
    assert_portable_object_matches(
        b"definitely not an object",
        b"definitely not an object",
        "t",
    );
}

#[test]
#[should_panic(expected = "portable object bytes mismatch: fixture-x")]
fn mismatch_panics_with_caller_context() {
    assert_portable_object_matches(b"left", b"right", "fixture-x");
}

#[test]
#[should_panic(expected = "bytes: ")]
fn mismatch_report_includes_bounded_diff_body() {
    assert_portable_object_matches(b"left", b"right", "fixture-y");
}

#[test]
fn macro_form_passes_optional_context() {
    assert_portable_object_matches!(b"same", b"same");
    assert_portable_object_matches!(b"same", b"same", "with-context");
}
