//! 正式文档的 TOML 页头；字段类型与取值是唯一机器模型。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    pub schema_version: u32,
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applicability: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<Replacement>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Kind {
    Index,
    Adr,
    Design,
    Guide,
    Reference,
    Governance,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Status {
    Draft,
    Review,
    Accepted,
    Rejected,
    Active,
    Retired,
}

impl Status {
    pub fn effective(self) -> bool {
        matches!(self, Self::Accepted | Self::Active | Self::Retired)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "草案",
            Self::Review => "待审阅",
            Self::Accepted => "已接受",
            Self::Rejected => "已拒绝",
            Self::Active => "持续维护",
            Self::Retired => "已退役",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Replacement {
    pub target: String,
    pub extent: Extent,
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Extent {
    Full,
    Partial,
}

/// 分隔符只在行首独占一行；原始偏移保留给 Markdown 诊断。
pub(super) fn split(text: &str) -> Result<(&str, &str, usize), String> {
    let first = text.find('\n').ok_or("缺少 TOML 页头")?;
    if text[..first].trim_end_matches('\r') != "+++" {
        return Err("正式文档必须以 +++ TOML 页头开始".into());
    }
    let start = first + 1;
    let mut offset = start;
    for line in text[start..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "+++" {
            let end = offset + line.len();
            return Ok((&text[start..offset], &text[end..], end));
        }
        offset += line.len();
    }
    Err("TOML 页头缺少结束 +++".into())
}

/// 只屏蔽完整的 TOML 页头，不把 Skill 的 YAML 误认为正式文档元数据。
pub(super) fn mask(text: &str) -> std::borrow::Cow<'_, str> {
    let Ok((_, _, end)) = split(text) else {
        return text.into();
    };
    let mut bytes = text.as_bytes().to_vec();
    for byte in &mut bytes[..end] {
        if !matches!(*byte, b'\n' | b'\r') {
            *byte = b' ';
        }
    }
    // 屏蔽区每字节替换为 ASCII，正文仍为原 UTF-8。
    String::from_utf8(bytes).expect("masked UTF-8").into()
}

pub(super) fn parse(text: &str) -> Result<Metadata, String> {
    let (header, _, _) = split(text)?;
    let metadata: Metadata =
        toml::from_str(header).map_err(|e| format!("TOML 页头无效（页头内行号）：{e}"))?;
    metadata.validate()?;
    Ok(metadata)
}

pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn plain(value: &str) -> bool {
    !value.trim().is_empty() && value == value.trim() && !value.chars().any(char::is_control)
}

pub(super) fn date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| !matches!(i, 4 | 7) && !b.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year > 0 && day > 0 && day <= max
}

impl Metadata {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("schema_version 仅支持 1".into());
        }
        if !identifier(&self.id) {
            return Err("id 必须是小写 ASCII 字母/数字组成的连字符分隔标识".into());
        }
        let allowed = match self.kind {
            Kind::Adr => matches!(
                self.status,
                Status::Draft | Status::Review | Status::Accepted | Status::Rejected
            ),
            Kind::Design => self.status != Status::Rejected,
            _ => matches!(
                self.status,
                Status::Draft | Status::Review | Status::Active | Status::Retired
            ),
        };
        if !allowed {
            return Err("kind 与 status 不匹配".into());
        }
        if !plain(&self.summary) || self.summary.contains(['[', ']', '`', '<', '>', '*', '_']) {
            return Err("summary 必须是非空单行纯文本（不用 Markdown 标记）".into());
        }
        if self.kind != Kind::Index && self.scope.is_none() {
            return Err("非索引文档必须有 scope".into());
        }
        if self.scope.as_deref().is_some_and(|s| !plain(s)) {
            return Err("scope 必须是非空单行文本".into());
        }
        for value in [
            &self.date,
            &self.revised_date,
            &self.decision_date,
            &self.verified_date,
        ]
        .into_iter()
        .flatten()
        {
            if !date(value) {
                return Err(format!("日期必须是有效的 YYYY-MM-DD 字符串：{value}"));
            }
        }
        if self.decision_date.is_some()
            && !matches!(self.status, Status::Accepted | Status::Rejected)
        {
            return Err("decision_date 仅用于已接受或已拒绝的决策".into());
        }
        if self.verified_date.is_some() != self.verified_revision.is_some() {
            return Err("verified_date 与 verified_revision 必须成对".into());
        }
        if let Some(revision) = &self.verified_revision
            && (revision.len() != 40
                || !revision
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        {
            return Err("verified_revision 必须是 40 位小写 Git 提交 ID".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for issue in &self.issues {
            let valid = issue
                .strip_prefix("https://github.com/")
                .is_some_and(|tail| {
                    let parts: Vec<_> = tail.split('/').collect();
                    parts.len() == 4
                        && parts[..2].iter().all(|p| {
                            !p.is_empty()
                                && p.bytes().all(|b| {
                                    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
                                })
                        })
                        && matches!(parts[2], "issues" | "pull")
                        && parts[3].parse::<u64>().is_ok_and(|n| n > 0)
                });
            if !valid || !seen.insert(issue) {
                return Err(format!("议题链接无效或重复：{issue}"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for target in &self.related {
            if !identifier(target) || target == &self.id || !seen.insert(target) {
                return Err(format!("related 无效、自引用或重复：{target}"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for edge in &self.supersedes {
            if !identifier(&edge.target) || edge.target == self.id || !seen.insert(&edge.target) {
                return Err(format!("supersedes 无效、自引用或重复：{}", edge.target));
            }
            fragment(&edge.explanation)?;
        }
        if let Some(value) = &self.applicability {
            fragment(value)?;
        }
        Ok(())
    }
}

fn fragment(value: &str) -> Result<(), String> {
    if value.strip_prefix('#').is_some_and(identifier) {
        Ok(())
    } else {
        Err(format!("范围入口必须引用本页稳定 ASCII 锚点：{value}"))
    }
}
