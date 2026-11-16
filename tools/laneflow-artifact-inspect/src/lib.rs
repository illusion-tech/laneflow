//! LFCA / LFSM / LFSD / LFCP 制品的只读检视、结构化差异与 fixture 断言辅助。
//!
//! 本 crate 是库 + CLI：库承载输入解码、借用遍历、结构比较与有界渲染，
//! CLI（`main.rs`）承载参数、文件读取、输出与退出码。检查语义复用
//! `laneflow-format` 的正式预检与登记：工具不复制线格式偏移常量，不新增
//! 独立 lf* 解析器，也不放宽任何检查。
//!
//! 检查范围声明：本工具执行按登记、按逻辑位置的单对象结构比较；跨对象绑定、
//! 运行时静态路网构建与发布真实性不在检查范围内。诊断文本不是数据契约。

pub mod budget;
pub mod diff;
pub mod input;
pub mod inspect;
pub mod render;
pub mod value;
pub mod walk;

#[cfg(test)]
mod lib_tests;

use std::fmt::Write as _;

/// 断言实际发射字节与期望字节一致；失配时给出有界结构化诊断。
///
/// 相等路径立即返回：不解码、不构造诊断字符串、不调用摘要探针。不相等时才
/// 执行有界诊断（按 [`diff`] 的三类差异）并以 `context` 标注本次比较的
/// fixture 与重生成提示；`context` 由测试调用方或包装层提供，本库不猜测
/// dump 命令。panic 位置指向测试调用处（`#[track_caller]`）。
///
/// 本辅助只承担字节一致性断言，不替代单对象有效性检查；预检失败也会让
/// 不相等的字节失败，但诊断会同时报告两侧检查状态。
///
/// # Panics
///
/// `actual` 与 `expected` 字节不一致时 panic，消息包含有界差异报告。
#[track_caller]
pub fn assert_portable_object_matches(actual: &[u8], expected: &[u8], context: &str) {
    if actual == expected {
        return;
    }
    let options = diff::DiffOptions::default();
    let report = diff::diff_bytes(expected, actual);
    let mut message = String::new();
    let _ = writeln!(message, "portable object bytes mismatch: {context}");
    render::render_diff_into(
        &mut message,
        &report,
        &options,
        &budget::ReportLimits::default(),
    );
    panic!("{message}");
}

/// [`assert_portable_object_matches`] 的宏形式；第三个参数（fixture 标识与
/// 重生成提示）可省略，省略时以 `<unset>` 标注。
///
/// ```no_run
/// # let actual: &[u8] = &[]; let expected: &[u8] = &[];
/// laneflow_artifact_inspect::assert_portable_object_matches!(actual, expected);
/// laneflow_artifact_inspect::assert_portable_object_matches!(
///     actual,
///     expected,
///     "lfca-variants/min-headless.lfca（DUMP_PORTABLE 流程重生成）"
/// );
/// ```
#[macro_export]
macro_rules! assert_portable_object_matches {
    ($actual:expr, $expected:expr $(,)?) => {
        $crate::assert_portable_object_matches($actual, $expected, "<unset>")
    };
    ($actual:expr, $expected:expr, $context:expr $(,)?) => {
        $crate::assert_portable_object_matches($actual, $expected, $context)
    };
}
