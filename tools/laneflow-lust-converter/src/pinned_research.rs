//! 冻结研究数据的运行时读取（仅测试）。数据文件已按 #1249 / #1364 退仓出源码树，
//! 根 `research/` 目录随 #1378 整体移除；`#[ignore]` 手动重放实验经
//! `PINNED_RESEARCH_ROOT` 指向含数据冻结提交（如 `564f9ac8`、`f71123b5`）checkout
//! 的 `research/` 目录读取，不回落到当前树。
//!
//! 出处约定：本 crate 注释、测试 fixture 与输出来源字段（`cohort_source`、
//! `method_source`、`coordination`、`baseline_pool` 等）中的 `research/...` 相对路径
//! 均以冻结提交 checkout 为根：计划、设计与各研究目录 `README.md` 在 `e267d4c9`
//! （最后一个包含该目录的 main 提交）可查，数据文件在上述含数据冻结提交可查。
//! 输出来源字段属于冻结输出身份，不随目录移除改写。

/// 读取退仓的冻结研究数据文本。
///
/// # Panics
/// `PINNED_RESEARCH_ROOT` 未设置，或该目录下不存在目标文件。
pub(crate) fn text(relative: &str) -> String {
    let root = std::env::var("PINNED_RESEARCH_ROOT").unwrap_or_else(|_| {
        panic!(
            "PINNED_RESEARCH_ROOT 未设置：冻结研究数据已退仓（#1249/#1364），须指向含数据冻结提交（如 564f9ac8 或 f71123b5；各研究 README 在 e267d4c9 可查）checkout 的 research/ 目录"
        )
    });
    std::fs::read_to_string(std::path::Path::new(&root).join(relative))
        .unwrap_or_else(|error| panic!("读取冻结研究数据 `{relative}` 失败：{error}"))
}
