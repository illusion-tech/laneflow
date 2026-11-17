//! 术语 TOML 正本、生成阅读页与网站数据共用的内容模型。

mod model;
mod render;
mod validate;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::catalog::Catalog;
use super::scan::{self, Stat};
pub(crate) use model::TermCatalog;
use model::{Located, Manifest, Source};

const DIRECTORY: &str = "docs/reference/terms";
const GLOSSARY: &str = "docs/reference/glossary.md";
const WORDING: &str = "docs/reference/wording-guide.md";
const BEGIN: &str = "<!-- generated-terminology:start -->";
const END: &str = "<!-- generated-terminology:end -->";

fn read(root: &Path, relative: &str) -> Result<String, String> {
    if !matches!(scan::stat_rel(root, relative), Ok(Stat::File)) {
        return Err(format!(
            "术语来源必须是仓库内普通文件，不接受符号链接：{relative}"
        ));
    }
    fs::read_to_string(root.join(relative)).map_err(|e| format!("{relative}: {e}"))
}

impl TermCatalog {
    pub(super) fn load(root: &Path, documents: &Catalog) -> Result<Self, String> {
        let path = format!("{DIRECTORY}/catalog.toml");
        let manifest: Manifest =
            toml::from_str(&read(root, &path)?).map_err(|e| format!("{path}: {e}"))?;
        validate::manifest(&manifest)?;
        let mut actual = BTreeSet::new();
        for entry in fs::read_dir(root.join(DIRECTORY)).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "术语路径须为 UTF-8")?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err(format!("术语目录只允许普通 TOML 文件：{name}"));
            }
            if name != "catalog.toml" {
                actual.insert(name);
            }
        }
        if actual != manifest.files.iter().cloned().collect() {
            return Err("术语目录文件与 catalog.toml 的 files 不一致".into());
        }
        let mut result = Self {
            schema_version: 1,
            domains: manifest.domains,
            terms: Vec::new(),
            name_changes: Vec::new(),
            wording_rules: Vec::new(),
        };
        for file in manifest.files {
            let source = format!("{DIRECTORY}/{file}");
            let data: Source =
                toml::from_str(&read(root, &source)?).map_err(|e| format!("{source}: {e}"))?;
            if data.schema_version != 1 {
                return Err(format!("{source}: 术语 schema_version 仅支持 1"));
            }
            if data.terms.is_empty()
                && data.name_changes.is_empty()
                && data.wording_rules.is_empty()
            {
                return Err(format!("{source}: 术语来源不得为空"));
            }
            result
                .terms
                .extend(data.terms.into_iter().map(|record| Located {
                    url: format!("/docs/reference-glossary/#term-{}", record.id),
                    source: source.clone(),
                    record,
                }));
            result
                .name_changes
                .extend(data.name_changes.into_iter().map(|record| Located {
                    url: format!("/docs/reference-glossary/#name-{}", record.id),
                    source: source.clone(),
                    record,
                }));
            result
                .wording_rules
                .extend(data.wording_rules.into_iter().map(|record| Located {
                    url: format!("/docs/reference-wording-guide/#wording-{}", record.id),
                    source: source.clone(),
                    record,
                }));
        }
        result.terms.sort_by(|a, b| a.record.id.cmp(&b.record.id));
        result
            .name_changes
            .sort_by(|a, b| a.record.id.cmp(&b.record.id));
        result
            .wording_rules
            .sort_by(|a, b| a.record.id.cmp(&b.record.id));
        result.validate(documents)?;
        Ok(result)
    }

    pub(super) fn pages(
        &self,
        root: &Path,
        documents: &Catalog,
        write: bool,
    ) -> Result<(), String> {
        let rendered = [
            (GLOSSARY, self.glossary(documents)),
            (WORDING, self.wording(documents)),
        ];
        let mut changes = Vec::new();
        for (path, body) in rendered {
            let original = read(root, path)?.replace("\r\n", "\n");
            let expected = replace_region(&original, &body).map_err(|e| format!("{path}: {e}"))?;
            // 生成正文也经过现有 Markdown 解析器；名称改动不能制造锚点冲突。
            let parsed = super::markdown::parse_document(&expected, path);
            if !parsed.duplicate_anchors.is_empty() || parsed.titles.len() != 1 {
                return Err(format!("{path}: 生成内容出现标题或锚点冲突"));
            }
            if original != expected {
                changes.push((path, expected));
            }
        }
        if !write && !changes.is_empty() {
            return Err(format!(
                "术语阅读页已漂移，运行 generate-glossary：{}",
                changes
                    .iter()
                    .map(|(p, _)| *p)
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        // 两份页面的模型、引用和生成区域全部通过后才写入。
        for (path, expected) in changes {
            fs::write(root.join(path), expected).map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(())
    }
}

fn replace_region(source: &str, body: &str) -> Result<String, String> {
    if source.matches(BEGIN).count() != 1 || source.matches(END).count() != 1 {
        return Err("必须恰有一组术语生成区域标记".into());
    }
    let start = source.find(BEGIN).ok_or("缺少生成起点")?;
    let end = source.find(END).ok_or("缺少生成终点")? + END.len();
    if start >= end || !source[end..].trim().is_empty() {
        return Err("术语生成区必须位于文件末尾，标记顺序正确".into());
    }
    for (marker, position) in [(BEGIN, start), (END, end - END.len())] {
        let after = &source[position + marker.len()..];
        let standalone = (position == 0 || source[..position].ends_with('\n'))
            && (after.is_empty() || after.starts_with('\n'));
        let comment =
            pulldown_cmark::Parser::new(source)
                .into_offset_iter()
                .any(|(event, range)| {
                    matches!(event, pulldown_cmark::Event::Html(_))
                        && range.start <= position
                        && range.end >= position + marker.len()
                });
        if !standalone || !comment {
            return Err("生成标记须为正文中独占一行的 HTML 注释，不能位于代码块".into());
        }
    }
    Ok(format!("{}{BEGIN}\n\n{body}\n{END}\n", &source[..start]))
}

/// 生成术语与措辞阅读页，或检查生成区是否与 TOML 正本一致。
///
/// # Errors
///
/// 参数、来源、字段、引用、名称或生成区域无效，以及文件读写失败时返回错误。
pub(crate) fn generate(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("用法：generate-glossary [--check]".into()),
    };
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let documents = Catalog::load(&root)?;
    let terms = TermCatalog::load(&root, &documents)?;
    terms.pages(&root, &documents, !check)?;
    println!(
        "术语 {}；旧称 {}；措辞规则 {}；阅读页{}",
        terms.terms.len(),
        terms.name_changes.len(),
        terms.wording_rules.len(),
        if check { "一致" } else { "已更新" }
    );
    Ok(())
}
