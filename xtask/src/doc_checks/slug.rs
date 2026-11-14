//! 有限的 GitHub 兼容标题 slug。
//!
//! 不声称复现 GitHub 渲染器。正反例见 `docs/reference/documentation-checks.md`。

use std::collections::BTreeMap;

/// 从标题可见文本生成 slug。
///
/// 先去掉首尾空白，再只把 ASCII `A-Z` 收成小写。连字符与下划线保留，
/// Unicode 字母和数字（含中文）保留。每个空白字符各映射为一个 `-`，不合并；
/// 其余字符删除。夹在空白之间的标点删掉后，两侧空白各留一个 `-`，因此
/// `A6 = 选项` 变成 `a6--选项`，`a  b` 变成 `a--b`。
pub(super) fn github_subset_slug(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.trim().chars() {
        let ch = if ch.is_ascii_uppercase() {
            ch.to_ascii_lowercase()
        } else {
            ch
        };
        if ch.is_whitespace() {
            slug.push('-');
        } else if ch == '-' || ch == '_' || ch.is_alphanumeric() {
            slug.push(ch);
        }
    }
    slug
}

/// 全文件共享的 slug 命名空间。重复标题追加 `-1`、`-2`，并避开已被占用的后缀。
#[derive(Debug, Default)]
pub(super) struct Slugger {
    seen: BTreeMap<String, u32>,
}

impl Slugger {
    pub(super) fn next(&mut self, text: &str) -> String {
        let base = github_subset_slug(text);
        let mut candidate = base.clone();
        while self.seen.contains_key(&candidate) {
            let next = self.seen.get(&base).copied().unwrap_or(0).saturating_add(1);
            self.seen.insert(base.clone(), next);
            candidate = format!("{base}-{next}");
        }
        self.seen.insert(candidate.clone(), 0);
        candidate
    }
}
