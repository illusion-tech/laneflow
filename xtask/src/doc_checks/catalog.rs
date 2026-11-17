//! 从正式源文档构建唯一目录；索引与网站数据消费同一份类型化结果。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use super::markdown::{self, LinkKind};
use super::metadata::{self, Extent, Kind, Metadata};
use super::scan;

const BEGIN: &str = "<!-- generated-doc-index:start -->";
const END: &str = "<!-- generated-doc-index:end -->";

#[derive(Serialize)]
pub(super) struct Page {
    pub metadata: Metadata,
    pub path: String,
    pub title: String,
    pub url: String,
    pub validity: Validity,
    pub replaced_by: Vec<String>,
    pub anchors: BTreeSet<String>,
    pub headings: Vec<markdown::Heading>,
    pub links: Vec<LocalLink>,
    pub body: String,
    #[serde(skip)]
    source: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Validity {
    Unreplaced,
    PartiallySuperseded,
    Superseded,
}

impl Validity {
    fn label(self) -> &'static str {
        match self {
            Self::Unreplaced => "无生效取代",
            Self::PartiallySuperseded => "部分取代",
            Self::Superseded => "全部取代",
        }
    }
}

#[derive(Serialize)]
pub(super) struct LocalLink {
    pub raw: String,
    pub target: String,
    pub directory: bool,
    pub fragment: Option<String>,
    pub image: bool,
    pub page_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Catalog {
    schema_version: u32,
    pub(super) pages: Vec<Page>,
}

impl Catalog {
    pub(super) fn load(root: &Path) -> Result<Self, String> {
        let mut paths = Vec::new();
        collect(root, "docs", &mut paths)?;
        paths.sort();
        if paths.is_empty() {
            return Err("docs 中没有正式文档".into());
        }
        let mut pages = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let source =
                fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            match page(&path, source) {
                Ok(page) => pages.push(page),
                Err(error) => errors.push(format!("{path}:1:1: metadata: {error}")),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        let mut result = Self {
            schema_version: 1,
            pages,
        };
        result.validate_graph()?;
        result.resolve_links();
        Ok(result)
    }

    fn validate_graph(&mut self) -> Result<(), String> {
        let mut ids = BTreeMap::new();
        for (index, page) in self.pages.iter().enumerate() {
            if let Some(previous) = ids.insert(page.metadata.id.clone(), index) {
                return Err(format!(
                    "重复文档 ID {}：{} / {}",
                    page.metadata.id, self.pages[previous].path, page.path
                ));
            }
        }
        let mut incoming: BTreeMap<usize, Vec<(usize, Extent)>> = BTreeMap::new();
        for (index, page) in self.pages.iter().enumerate() {
            for related in &page.metadata.related {
                if !ids.contains_key(related) {
                    return Err(format!("{}: related 目标不存在：{related}", page.path));
                }
            }
            for edge in &page.metadata.supersedes {
                let Some(&target) = ids.get(&edge.target) else {
                    return Err(format!("{}: 取代目标不存在：{}", page.path, edge.target));
                };
                if page.metadata.status.effective() {
                    incoming
                        .entry(target)
                        .or_default()
                        .push((index, edge.extent));
                }
            }
        }
        // 提议中的边也不得形成循环；它们不参与当前有效性计算。
        for start in 0..self.pages.len() {
            let mut pending: Vec<_> = self.pages[start]
                .metadata
                .supersedes
                .iter()
                .map(|e| ids[&e.target])
                .collect();
            let mut seen = BTreeSet::new();
            while let Some(index) = pending.pop() {
                if index == start {
                    return Err(format!(
                        "取代关系形成循环：{}",
                        self.pages[start].metadata.id
                    ));
                }
                if seen.insert(index) {
                    pending.extend(
                        self.pages[index]
                            .metadata
                            .supersedes
                            .iter()
                            .map(|e| ids[&e.target]),
                    );
                }
            }
        }
        for (target, edges) in incoming {
            if edges
                .iter()
                .filter(|(_, extent)| *extent == Extent::Full)
                .count()
                > 1
            {
                return Err(format!(
                    "{} 有多个直接全部取代来源；应通过后继决策串联",
                    self.pages[target].metadata.id
                ));
            }
            let validity = if edges.iter().any(|(_, extent)| *extent == Extent::Full) {
                Validity::Superseded
            } else {
                Validity::PartiallySuperseded
            };
            let mut replaced_by: Vec<_> = edges
                .iter()
                .map(|(source, _)| self.pages[*source].metadata.id.clone())
                .collect();
            replaced_by.sort();
            self.pages[target].validity = validity;
            self.pages[target].replaced_by = replaced_by;
        }
        Ok(())
    }

    fn resolve_links(&mut self) {
        let targets: BTreeMap<_, _> = self
            .pages
            .iter()
            .map(|p| (p.path.clone(), (p.metadata.id.clone(), p.url.clone())))
            .collect();
        for page in &mut self.pages {
            for link in &mut page.links {
                let target = if link.directory {
                    format!("{}/README.md", link.target)
                } else {
                    link.target.clone()
                };
                if let Some((id, url)) = targets.get(&target) {
                    link.page_id = Some(id.clone());
                    link.url = Some(format!(
                        "{url}{}",
                        link.fragment
                            .as_ref()
                            .map_or(String::new(), |f| format!("#{f}"))
                    ));
                }
            }
        }
    }

    pub(super) fn indexes(&self, root: &Path, write: bool) -> Result<(), String> {
        let mut changes = Vec::new();
        for page in self.pages.iter().filter(|p| p.metadata.kind == Kind::Index) {
            let region = self.index_region(page);
            if page.source.matches(BEGIN).count() != 1 || page.source.matches(END).count() != 1 {
                return Err(format!("{}: 索引必须恰有一组生成区域标记", page.path));
            }
            let start = page.source.find(BEGIN).ok_or("索引缺少起点")?;
            let end = page.source.find(END).ok_or("索引缺少终点")? + END.len();
            if end <= start || !page.source[end..].trim().is_empty() {
                return Err(format!(
                    "{}: 生成区域必须在文件末尾且标记顺序正确",
                    page.path
                ));
            }
            let expected = format!("{}{}\n", &page.source[..start], region);
            if page.source.replace("\r\n", "\n") != expected.replace("\r\n", "\n") {
                changes.push((page.path.clone(), expected));
            }
        }
        if !write && !changes.is_empty() {
            return Err(format!(
                "索引已漂移，运行 generate-doc-indexes：\n{}",
                changes
                    .iter()
                    .map(|(p, _)| p.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        // 全部结构验证成功后才写，避免遇到坏标记时留下部分更新。
        for (path, contents) in changes {
            fs::write(root.join(&path), contents).map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(())
    }

    fn index_region(&self, index: &Page) -> String {
        let parent = scan::parent_rel(&index.path);
        let mut members: Vec<_> = self
            .pages
            .iter()
            .filter(|p| {
                p.path != index.path
                    && (scan::parent_rel(&p.path) == parent
                        || (p.metadata.kind == Kind::Index
                            && scan::parent_rel(scan::parent_rel(&p.path)) == parent))
            })
            .collect();
        members.sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
        let mut text = format!("{BEGIN}\n\n");
        for p in &members {
            text.push_str(&format!(
                "- [{}][doc-{}] — {}；{}。{}\n",
                escape_label(&p.title),
                p.metadata.id,
                p.metadata.status.label(),
                p.validity.label(),
                p.metadata.summary
            ));
        }
        text.push('\n');
        for p in &members {
            let relative = p
                .path
                .strip_prefix(&format!("{parent}/"))
                .unwrap_or(&p.path);
            text.push_str(&format!("[doc-{}]: {}\n", p.metadata.id, relative));
        }
        text.push_str(&format!("\n{END}"));
        text
    }
}

fn escape_label(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn collect(root: &Path, relative: &str, paths: &mut Vec<String>) -> Result<(), String> {
    let directory = root.join(relative);
    if fs::symlink_metadata(&directory)
        .map_err(|e| format!("{relative}: {e}"))?
        .file_type()
        .is_symlink()
    {
        return Err(format!("正式文档路径不允许符号链接：{relative}"));
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "文档路径必须是 UTF-8")?;
        let path = format!("{relative}/{name}");
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            return Err(format!("正式文档路径不允许符号链接：{path}"));
        }
        if ty.is_dir()
            && !matches!(
                name.as_str(),
                "fixtures" | "tests" | "target" | "node_modules" | ".git"
            )
        {
            collect(root, &path, paths)?;
        } else if ty.is_file() && name.ends_with(".md") {
            paths.push(path);
        }
    }
    Ok(())
}

fn page(path: &str, source: String) -> Result<Page, String> {
    let metadata = metadata::parse(&source)?;
    let (_, body, _) = metadata::split(&source)?;
    let parsed = markdown::parse_document(&source, path);
    if !parsed.duplicate_anchors.is_empty() {
        return Err(format!("锚点冲突：{:?}", parsed.duplicate_anchors));
    }
    if parsed.titles.len() != 1 || parsed.titles[0].trim().is_empty() {
        return Err("正文必须恰有一个非空一级标题".into());
    }
    if path.ends_with("/README.md") != (metadata.kind == Kind::Index) {
        return Err("docs 的 README.md 必须是 index，其他文件不得为 index".into());
    }
    if path.starts_with("docs/adr/") && metadata.kind != Kind::Index && metadata.kind != Kind::Adr {
        return Err("ADR 目录只接受 adr 或 index".into());
    }
    for anchor in metadata
        .applicability
        .iter()
        .chain(metadata.supersedes.iter().map(|e| &e.explanation))
    {
        if !parsed.anchors.contains(&anchor[1..]) {
            return Err(format!("范围入口不存在：{anchor}"));
        }
    }
    // 只检查文档页头，不把正文里的业务状态字段当作元数据。
    for line in body.lines().take_while(|line| !line.starts_with("## ")) {
        let line = line.trim().trim_start_matches('>').trim();
        if [
            "**状态**",
            "**文档状态**",
            "**最后更新**",
            "**最近修订**",
            "**日期**",
            "**适用范围**",
            "**最后核对**",
        ]
        .iter()
        .any(|key| line.starts_with(key))
        {
            return Err("页头仍存在旧加粗元数据字段".into());
        }
    }
    let links = parsed
        .links
        .into_iter()
        .filter_map(|l| match l.kind {
            LinkKind::Local {
                rel,
                fragment,
                directory,
            } => Some(LocalLink {
                raw: l.raw,
                target: rel,
                directory,
                fragment,
                image: l.image,
                page_id: None,
                url: None,
            }),
            _ => None,
        })
        .collect();
    Ok(Page {
        url: format!("/docs/{}/", metadata.id),
        metadata,
        path: path.into(),
        title: parsed.titles[0].clone(),
        validity: Validity::Unreplaced,
        replaced_by: Vec::new(),
        anchors: parsed.anchors,
        headings: parsed.headings,
        links,
        body: body.replace("\r\n", "\n"),
        source,
    })
}

/// 生成正式目录首页，或只检查生成区漂移。
///
/// # Errors
///
/// 参数、元数据、关系或索引标记无效，以及文件读写失败时返回错误。
pub(crate) fn generate(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("用法：generate-doc-indexes [--check]".into()),
    };
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let catalog = Catalog::load(&root)?;
    catalog.indexes(&root, !check)?;
    println!(
        "正式文档 {}；生成索引{}",
        catalog.pages.len(),
        if check { "一致" } else { "已更新" }
    );
    Ok(())
}

/// 检查后导出供网站适配器消费的规范化目录。
///
/// # Errors
///
/// 参数无效、完整文档检查失败、输出经过符号链接或文件读写失败时返回错误。
pub(crate) fn export(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("用法：export-docs（输出 target/docs/catalog.json）".into());
    }
    // 导出前复用完整检查，防止网站得到含断链或过期索引的数据。
    super::run(&[])?;
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let catalog = Catalog::load(&root)?;
    let terminology = super::terminology::TermCatalog::load(&root, &catalog)?;
    for component in ["target", "target/docs", "target/docs/catalog.json"] {
        match fs::symlink_metadata(root.join(component)) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("输出不能经过符号链接：{component}"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    fs::create_dir_all(root.join("target/docs")).map_err(|e| e.to_string())?;
    #[derive(Serialize)]
    struct Export<'a> {
        schema_version: u32,
        pages: &'a [Page],
        terminology: &'a super::terminology::TermCatalog,
    }
    let data = serde_json::to_string_pretty(&Export {
        schema_version: 2,
        pages: &catalog.pages,
        terminology: &terminology,
    })
    .map_err(|e| e.to_string())?;
    fs::write(root.join("target/docs/catalog.json"), format!("{data}\n"))
        .map_err(|e| e.to_string())?;
    println!("已导出 target/docs/catalog.json");
    Ok(())
}

#[cfg(test)]
mod tests;
