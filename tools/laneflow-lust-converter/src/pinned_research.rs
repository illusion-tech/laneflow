//! 冻结研究数据的运行时读取（仅测试）。数据文件已按 #1249 退仓出源码树；
//! `#[ignore]` 手动重放实验经 `PINNED_RESEARCH_ROOT` 指向含数据提交
//! （如 `564f9ac8`）checkout 的 `research/` 目录读取。

/// 读取退仓的冻结研究数据文本。
///
/// # Panics
/// `PINNED_RESEARCH_ROOT` 未设置，或该目录下不存在目标文件。
pub(crate) fn text(relative: &str) -> String {
    let root = std::env::var("PINNED_RESEARCH_ROOT").unwrap_or_else(|_| {
        panic!(
            "PINNED_RESEARCH_ROOT 未设置：冻结研究数据已退仓（#1249），             须指向含数据提交（如 564f9ac8）checkout 的 research/ 目录"
        )
    });
    std::fs::read_to_string(std::path::Path::new(&root).join(relative))
        .unwrap_or_else(|error| panic!("读取冻结研究数据 `{relative}` 失败：{error}"))
}
