use std::collections::{BTreeMap, BTreeSet};

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use super::model::*;
use crate::doc_checks::{catalog::Catalog, markdown, metadata};

fn require(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn plain(value: &str) -> bool {
    !value.is_empty() && value == value.trim() && !value.chars().any(char::is_control)
}

fn text(value: &str) -> bool {
    plain(value)
        && Parser::new_ext(value, markdown::parser_options()).all(|event| {
            matches!(
                event,
                Event::Start(Tag::Paragraph)
                    | Event::End(TagEnd::Paragraph)
                    | Event::Text(_)
                    | Event::Code(_)
            )
        })
}

fn strings(values: &[String]) -> bool {
    values.iter().all(|v| plain(v)) && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn notes(values: &[String]) -> bool {
    strings(values) && values.iter().all(|v| text(v))
}

pub(super) fn manifest(value: &Manifest) -> Result<(), String> {
    require(value.schema_version == 1, "术语 schema_version 仅支持 1")?;
    require(
        !value.files.is_empty() && strings(&value.files),
        "files 须为非空且无重复的文件列表",
    )?;
    for file in &value.files {
        require(
            file.strip_suffix(".toml").is_some_and(metadata::identifier) && file != "catalog.toml",
            format!("非法术语文件名：{file}"),
        )?;
    }
    let mut ids = BTreeSet::new();
    require(!value.domains.is_empty(), "术语领域不得为空")?;
    for domain in &value.domains {
        require(
            metadata::identifier(&domain.id) && ids.insert(&domain.id) && plain(&domain.name),
            format!("非法或重复术语领域：{}", domain.id),
        )?;
    }
    Ok(())
}

fn domains(values: &[String], allowed: &BTreeSet<&str>, context: &Option<String>) -> bool {
    !values.is_empty()
        && strings(values)
        && values.iter().all(|v| allowed.contains(v.as_str()))
        && context.as_deref().is_none_or(metadata::identifier)
}

fn references(values: &[Reference], documents: &Catalog) -> Result<(), String> {
    require(
        values.iter().collect::<BTreeSet<_>>().len() == values.len(),
        "责任文档引用重复",
    )?;
    for reference in values {
        let page = documents
            .pages
            .iter()
            .find(|p| p.metadata.id == reference.document)
            .ok_or_else(|| format!("责任文档不存在：{}", reference.document))?;
        require(
            !matches!(
                reference.document.as_str(),
                "reference-glossary" | "reference-wording-guide"
            ),
            "词条之间使用 related 或替换关系，不以生成页作为技术合同依据",
        )?;
        if let Some(anchor) = &reference.anchor {
            require(
                plain(anchor) && !anchor.contains('#') && page.anchors.contains(anchor),
                format!("责任文档锚点不存在：{}#{anchor}", reference.document),
            )?;
        }
    }
    Ok(())
}

fn issues(values: &[u32]) -> bool {
    values.iter().all(|v| *v != 0) && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

impl TermCatalog {
    pub(super) fn validate(&self, documents: &Catalog) -> Result<(), String> {
        let allowed: BTreeSet<_> = self.domains.iter().map(|d| d.id.as_str()).collect();
        let mut all_ids = BTreeMap::new();
        for (id, source) in self
            .terms
            .iter()
            .map(|t| (&t.record.id, &t.source))
            .chain(self.name_changes.iter().map(|t| (&t.record.id, &t.source)))
            .chain(self.wording_rules.iter().map(|t| (&t.record.id, &t.source)))
        {
            require(
                metadata::identifier(id),
                format!("{source}: 非法词条 ID：{id}"),
            )?;
            if let Some(previous) = all_ids.insert(id.as_str(), source) {
                return Err(format!("重复术语记录 ID {id}：{previous} / {source}"));
            }
        }
        require(!self.terms.is_empty(), "术语目录不得为空")?;
        let mut labels = BTreeMap::new();
        for located in &self.terms {
            let term = &located.record;
            let check = || -> Result<(), String> {
                require(
                    plain(&term.name) && text(&term.definition),
                    "名称或定义无效；定义仅允许单行文字与行内代码",
                )?;
                require(strings(&term.english_aliases), "英文辅助名无效或重复")?;
                require(
                    domains(&term.domains, &allowed, &term.context),
                    "领域或语境无效",
                )?;
                require(notes(&term.scope_notes), "范围说明无效或重复")?;
                require(
                    strings(&term.related) && !term.related.contains(&term.id),
                    "相关概念重复或自引用",
                )?;
                for related in &term.related {
                    require(
                        self.terms.iter().any(|t| t.record.id == *related),
                        format!("相关概念不存在：{related}"),
                    )?;
                }
                if let Some(borrowed) = &term.borrowed {
                    require(
                        borrowed.language == "en" && text(&borrowed.first_use_gloss),
                        "借词须声明 language = en 和中文首次释义",
                    )?;
                }
                require(
                    term.borrowed.is_some() || !term.english_aliases.is_empty(),
                    "非借词须有英文辅助名",
                )?;
                let mut symbols = BTreeSet::new();
                for symbol in &term.symbols {
                    require(
                        plain(&symbol.value) && symbols.insert((&symbol.value, symbol.kind)),
                        "符号为空或重复",
                    )?;
                }
                let mut short_names = BTreeSet::new();
                for short in &term.short_forms {
                    require(
                        plain(&short.name)
                            && text(&short.scope)
                            && short_names.insert(&short.name)
                            && short.name != term.name,
                        "简称须不同于规范名、唯一且具有适用语境",
                    )?;
                }
                references(&term.references, documents)?;
                if let Some(retirement) = &term.retirement {
                    require(
                        text(&retirement.reason) && !term.references.is_empty(),
                        "退役概念须说明原因及责任文档",
                    )?;
                    self.choices(&retirement.replacements)?;
                }
                Ok(())
            };
            check().map_err(|e| format!("{} [{}]: {e}", located.source, term.id))?;
            if term.retirement.is_none() {
                for name in std::iter::once(&term.name)
                    .chain(&term.english_aliases)
                    .chain(term.short_forms.iter().map(|s| &s.name))
                {
                    register(
                        &mut labels,
                        &term.domains,
                        term.context.as_deref(),
                        name,
                        &term.id,
                    )?;
                }
            }
        }
        for located in &self.name_changes {
            let change = &located.record;
            require(
                !change.names.is_empty()
                    && strings(&change.names)
                    && domains(&change.domains, &allowed, &change.context)
                    && text(&change.scope)
                    && notes(&change.exceptions)
                    && issues(&change.issues),
                format!(
                    "{} [{}]: 旧称范围、名称或登记无效",
                    located.source, change.id
                ),
            )?;
            require(
                !change.replacements.is_empty(),
                format!("{}: 旧称须有替换关系；废止概念使用 retirement", change.id),
            )?;
            self.choices(&change.replacements)?;
            for name in &change.names {
                register(
                    &mut labels,
                    &change.domains,
                    change.context.as_deref(),
                    name,
                    &change.id,
                )?;
            }
        }
        for located in &self.wording_rules {
            let rule = &located.record;
            require(
                !rule.expressions.is_empty()
                    && strings(&rule.expressions)
                    && text(&rule.scope)
                    && text(&rule.guidance)
                    && notes(&rule.exceptions)
                    && issues(&rule.issues),
                format!("{} [{}]: 措辞规则无效", located.source, rule.id),
            )?;
            references(&rule.references, documents)?;
            self.choices(&rule.alternatives)?;
        }
        for change in &self.name_changes {
            self.visit_change(
                &change.record.id,
                &mut BTreeSet::new(),
                &mut BTreeSet::new(),
            )?;
        }
        Ok(())
    }

    fn choices(&self, choices: &[Choice]) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for choice in choices {
            require(
                text(&choice.condition) && seen.insert((&choice.target, &choice.condition)),
                "替换条件为空、无效或重复",
            )?;
            require(
                self.terms
                    .iter()
                    .any(|t| t.record.id == choice.target && t.record.retirement.is_none())
                    || self
                        .name_changes
                        .iter()
                        .any(|n| n.record.id == choice.target),
                format!("替换目标不存在或已退役：{}", choice.target),
            )?;
        }
        Ok(())
    }

    fn visit_change<'a>(
        &'a self,
        id: &'a str,
        active: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Result<(), String> {
        if done.contains(id) {
            return Ok(());
        }
        require(active.insert(id), format!("旧称重定向循环：{id}"))?;
        if let Some(change) = self.name_changes.iter().find(|n| n.record.id == id) {
            for choice in &change.record.replacements {
                self.visit_change(&choice.target, active, done)?;
            }
        }
        active.remove(id);
        done.insert(id);
        Ok(())
    }
}

fn register<'a>(
    labels: &mut BTreeMap<(String, String, String), &'a str>,
    domains: &[String],
    context: Option<&str>,
    name: &str,
    id: &'a str,
) -> Result<(), String> {
    for domain in domains {
        let key = (
            domain.clone(),
            context.unwrap_or_default().to_string(),
            name.to_ascii_lowercase(),
        );
        if let Some(previous) = labels.insert(key, id) {
            require(
                previous == id,
                format!("同领域、同语境名称冲突：{name}（{previous} / {id}）"),
            )?;
        }
    }
    Ok(())
}
