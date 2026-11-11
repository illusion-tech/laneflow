//! `check-docs`：离线检查 Markdown 本地路径、标题锚点与关键索引入口。
//!
//! 仓库根是进程当前工作目录。检查器只读，不访问网络。

mod markdown;
mod scan;
mod slug;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use markdown::{Document, LinkKind, code_span_matches, parse_document};
use scan::{Stat, StatError};

/// 检查器入口。
///
/// # Errors
///
/// 参数无法识别、配置或源文件无法读取、路径越出仓库或是显式符号链接时返回错误。
/// 规则失败同样返回错误，正文是汇总行；具体诊断已写到 stderr。
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let paths = parse_args(args)?;
    let root = env::current_dir().map_err(|error| format!("无法取得当前工作目录: {error}"))?;
    let config = load_config(&root.join("xtask/data/doc-checks.toml"))?;
    let explicit = if paths.is_empty() { None } else { Some(paths) };
    let report = check(&root, explicit.as_deref(), &config)?;
    for finding in &report.findings {
        eprintln!("{finding}");
    }
    for note in &report.skip_notes {
        eprintln!("{note}");
    }
    if report.findings.is_empty() {
        println!("{}", report.summary());
        Ok(())
    } else {
        Err(report.summary())
    }
}

pub(crate) fn check(
    root: &Path,
    explicit: Option<&[PathBuf]>,
    config: &Config,
) -> Result<Report, String> {
    let sources = scan::collect_sources(root, explicit)?;
    let mut cache = DocCache::default();
    let mut findings = Vec::new();
    let mut skip_notes = Vec::new();
    let mut skips = BTreeMap::new();
    let mut links = 0_usize;

    for source in &sources {
        let document = cache.load(root, source)?;
        let owned_links = document.links.clone();
        for link in owned_links {
            links += 1;
            match judge_link(root, source, &link, &mut cache)? {
                Judgement::Ok => {}
                Judgement::Skip { kind, note } => {
                    *skips.entry(kind).or_insert(0) += 1;
                    if let Some(note) = note {
                        skip_notes.push(note);
                    }
                }
                Judgement::Finding(finding) => findings.push(finding),
            }
        }
    }

    for edge in &config.required {
        if let Some(finding) = judge_required(root, edge, &mut cache)? {
            findings.push(finding);
        }
    }

    findings.sort();
    skip_notes.sort();
    Ok(Report {
        sources: sources.len(),
        links,
        skips,
        findings,
        skip_notes,
    })
}

fn parse_args(args: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for arg in args {
        if arg.starts_with('-') {
            return Err(format!("未知 check-docs 参数：{arg}"));
        }
        paths.push(PathBuf::from(arg));
    }
    Ok(paths)
}

pub(crate) fn load_config(path: &Path) -> Result<Config, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "无法读取关键索引入口配置 `{}`: {error}。请在仓库根目录运行 check-docs",
            path.display()
        )
    })?;
    let file: ConfigFile =
        toml::from_str(&text).map_err(|error| format!("关键索引入口配置无效: {error}"))?;
    let mut required = Vec::with_capacity(file.required.len());
    for edge in file.required {
        required.push(RequiredEdge::parse(edge)?);
    }
    Ok(Config { required })
}

#[derive(Debug)]
pub(crate) struct Config {
    required: Vec<RequiredEdge>,
}

#[derive(Debug)]
struct RequiredEdge {
    source: String,
    path: String,
    fragment: Option<String>,
    display: String,
    accept_code_span: bool,
    reason: String,
}

impl RequiredEdge {
    fn parse(edge: RequiredEdgeFile) -> Result<Self, String> {
        let reason = edge.reason.trim();
        if reason.is_empty() {
            return Err("关键索引入口缺少 reason".to_string());
        }
        if !is_config_path(&edge.source) || !scan::is_markdown_rel(&edge.source) {
            return Err(format!(
                "关键索引入口 source 必须是仓库内 Markdown 相对路径：`{}`",
                edge.source
            ));
        }
        let (path, fragment) = split_config_target(&edge.target)?;
        if !is_config_path(path) {
            return Err(format!(
                "关键索引入口 target 必须是仓库内相对路径：`{}`",
                edge.target
            ));
        }
        Ok(Self {
            source: edge.source,
            path: path.to_string(),
            fragment: fragment.map(str::to_string),
            display: edge.target,
            accept_code_span: edge.accept_code_span,
            reason: reason.to_string(),
        })
    }
}

fn is_config_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains('#')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn split_config_target(target: &str) -> Result<(&str, Option<&str>), String> {
    match target.split_once('#') {
        Some((path, fragment)) if !fragment.is_empty() && !fragment.contains('#') => {
            Ok((path, Some(fragment)))
        }
        Some(_) => Err(format!("关键索引入口 target 的 fragment 无效：`{target}`")),
        None => Ok((target, None)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    required: Vec<RequiredEdgeFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredEdgeFile {
    source: String,
    target: String,
    #[serde(default)]
    accept_code_span: bool,
    reason: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FindingKind {
    Path,
    Anchor,
    RequiredEntry,
    Inline,
}

impl FindingKind {
    fn token(self) -> &'static str {
        match self {
            Self::Path => "missing-path",
            Self::Anchor => "missing-anchor",
            Self::RequiredEntry => "missing-required-entry",
            Self::Inline => "inline-form",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum SkipKind {
    Http,
    Mailto,
    Rustdoc,
    Unsupported,
    NonMarkdownFragment,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Finding {
    path: String,
    line: usize,
    column: usize,
    kind: FindingKind,
    target: String,
    reason: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: target={}: {}: {}",
            self.path,
            self.line,
            self.column,
            self.target,
            self.kind.token(),
            self.reason
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SkipNote {
    path: String,
    line: usize,
    column: usize,
    target: String,
    reason: String,
}

impl fmt::Display for SkipNote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: target={}: skip: {}",
            self.path, self.line, self.column, self.target, self.reason
        )
    }
}

#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) sources: usize,
    pub(crate) links: usize,
    pub(crate) skips: BTreeMap<SkipKind, usize>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) skip_notes: Vec<SkipNote>,
}

impl Report {
    pub(crate) fn summary(&self) -> String {
        let skipped: usize = self.skips.values().copied().sum();
        let count = |kind| self.skips.get(&kind).copied().unwrap_or(0);
        format!(
            "check-docs: sources={} links={} skipped={skipped} errors={}\nskipped-by-type: http={} mailto={} rustdoc={} unsupported={} non-markdown-fragment={}",
            self.sources,
            self.links,
            self.findings.len(),
            count(SkipKind::Http),
            count(SkipKind::Mailto),
            count(SkipKind::Rustdoc),
            count(SkipKind::Unsupported),
            count(SkipKind::NonMarkdownFragment),
        )
    }
}

#[derive(Default)]
struct DocCache {
    docs: BTreeMap<String, Document>,
}

impl DocCache {
    fn load(&mut self, root: &Path, rel: &str) -> Result<&Document, String> {
        if !self.docs.contains_key(rel) {
            let path = scan::abs_from_rel(root, rel);
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("无法读取 Markdown 文件 `{}`: {error}", path.display()))?;
            self.docs
                .insert(rel.to_string(), parse_document(&text, rel));
        }
        self.docs
            .get(rel)
            .ok_or_else(|| format!("无法读取 Markdown 文件 `{rel}`"))
    }
}

enum Judgement {
    Ok,
    Skip {
        kind: SkipKind,
        note: Option<SkipNote>,
    },
    Finding(Finding),
}

fn judge_link(
    root: &Path,
    source: &str,
    link: &markdown::LinkRec,
    cache: &mut DocCache,
) -> Result<Judgement, String> {
    match &link.kind {
        LinkKind::Skip {
            kind,
            reason,
            visible,
        } => Ok(Judgement::Skip {
            kind: *kind,
            note: visible.then(|| SkipNote {
                path: source.to_string(),
                line: link.line,
                column: link.column,
                target: link.raw.clone(),
                reason: reason.clone(),
            }),
        }),
        LinkKind::Failure { kind, reason } => Ok(Judgement::Finding(finding(
            source,
            link.line,
            link.column,
            *kind,
            &link.raw,
            (*reason).to_string(),
        ))),
        LinkKind::Local {
            rel,
            directory,
            fragment,
        } => judge_local(
            root,
            source,
            link,
            rel,
            *directory,
            fragment.as_deref(),
            cache,
        ),
    }
}

fn judge_local(
    root: &Path,
    source: &str,
    link: &markdown::LinkRec,
    rel: &str,
    directory: bool,
    fragment: Option<&str>,
    cache: &mut DocCache,
) -> Result<Judgement, String> {
    let located = match scan::stat_rel(root, rel) {
        Ok(stat) => stat,
        Err(StatError::Missing) => {
            return Ok(Judgement::Finding(finding(
                source,
                link.line,
                link.column,
                FindingKind::Path,
                &link.raw,
                "本地路径不存在".to_string(),
            )));
        }
        Err(StatError::Symlink) => {
            return Ok(Judgement::Finding(finding(
                source,
                link.line,
                link.column,
                FindingKind::Path,
                &link.raw,
                "不跟随符号链接".to_string(),
            )));
        }
        Err(StatError::NotDirectory) => {
            return Ok(Judgement::Finding(finding(
                source,
                link.line,
                link.column,
                FindingKind::Path,
                &link.raw,
                "路径不是目录".to_string(),
            )));
        }
        Err(StatError::Special) => {
            return Ok(Judgement::Finding(finding(
                source,
                link.line,
                link.column,
                FindingKind::Path,
                &link.raw,
                "不支持的路径类型".to_string(),
            )));
        }
        Err(StatError::Io(message)) => return Err(message),
    };

    match located {
        Stat::Dir => {
            if fragment.is_some() {
                Ok(Judgement::Skip {
                    kind: SkipKind::Unsupported,
                    note: Some(SkipNote {
                        path: source.to_string(),
                        line: link.line,
                        column: link.column,
                        target: link.raw.clone(),
                        reason: "带 fragment 的目录链接不支持".to_string(),
                    }),
                })
            } else {
                Ok(Judgement::Ok)
            }
        }
        Stat::File if directory => Ok(Judgement::Finding(finding(
            source,
            link.line,
            link.column,
            FindingKind::Path,
            &link.raw,
            "带斜杠的路径不是目录".to_string(),
        ))),
        Stat::File if scan::is_markdown_rel(rel) => {
            let Some(fragment) = fragment else {
                return Ok(Judgement::Ok);
            };
            let present = cache.load(root, rel)?.anchors.contains(fragment);
            if present {
                Ok(Judgement::Ok)
            } else {
                Ok(Judgement::Finding(finding(
                    source,
                    link.line,
                    link.column,
                    FindingKind::Anchor,
                    &link.raw,
                    "目标 Markdown 中没有匹配的标题或显式 HTML 锚点".to_string(),
                )))
            }
        }
        Stat::File => {
            if fragment.is_some() {
                Ok(Judgement::Skip {
                    kind: SkipKind::NonMarkdownFragment,
                    note: None,
                })
            } else {
                Ok(Judgement::Ok)
            }
        }
    }
}

fn judge_required(
    root: &Path,
    edge: &RequiredEdge,
    cache: &mut DocCache,
) -> Result<Option<Finding>, String> {
    let source_is_markdown = match scan::stat_rel(root, &edge.source) {
        Err(StatError::Io(message)) => return Err(message),
        Ok(Stat::File) if scan::is_markdown_rel(&edge.source) => true,
        Ok(_) | Err(_) => false,
    };
    if !source_is_markdown {
        return Ok(Some(required_finding(
            edge,
            format!("源文件不存在；{}", edge.reason),
        )));
    }

    let document = cache.load(root, &edge.source)?;
    let referenced = document.links.iter().any(|link| link_hits(link, edge))
        || (edge.accept_code_span
            && document.code_spans.iter().any(|span| {
                code_span_matches(&edge.source, span, &edge.path, edge.fragment.as_deref())
            }));
    if !referenced {
        return Ok(Some(required_finding(edge, edge.reason.clone())));
    }

    match scan::stat_rel(root, &edge.path) {
        Err(StatError::Io(message)) => Err(message),
        Ok(Stat::File) if scan::is_markdown_rel(&edge.path) => {
            if let Some(fragment) = edge.fragment.as_deref() {
                let present = cache.load(root, &edge.path)?.anchors.contains(fragment);
                if !present {
                    return Ok(Some(required_finding(
                        edge,
                        format!("目标没有该锚点；{}", edge.reason),
                    )));
                }
            }
            Ok(None)
        }
        Ok(Stat::File) if edge.fragment.is_none() => Ok(None),
        Ok(_) | Err(_) => Ok(Some(required_finding(
            edge,
            format!("目标不存在；{}", edge.reason),
        ))),
    }
}

fn link_hits(link: &markdown::LinkRec, edge: &RequiredEdge) -> bool {
    if link.image {
        return false;
    }
    match &link.kind {
        LinkKind::Local {
            rel,
            fragment,
            directory,
        } => {
            !directory
                && rel == &edge.path
                && fragment_satisfies(edge.fragment.as_deref(), fragment.as_deref())
        }
        _ => false,
    }
}

fn fragment_satisfies(required: Option<&str>, actual: Option<&str>) -> bool {
    required == actual
}

fn finding(
    path: &str,
    line: usize,
    column: usize,
    kind: FindingKind,
    target: &str,
    reason: String,
) -> Finding {
    Finding {
        path: path.to_string(),
        line,
        column,
        kind,
        target: target.to_string(),
        reason,
    }
}

fn required_finding(edge: &RequiredEdge, reason: String) -> Finding {
    Finding {
        path: edge.source.clone(),
        line: 1,
        column: 1,
        kind: FindingKind::RequiredEntry,
        target: edge.display.clone(),
        reason,
    }
}
