use std::collections::BTreeMap;

use super::model::*;
use crate::doc_checks::catalog::Catalog;

fn label(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('#', "\\#")
        .replace('~', "\\~")
}

fn code(value: &str) -> String {
    let count = value.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
    let delimiter = "`".repeat(count);
    format!("{delimiter} {value} {delimiter}")
}

struct Links(BTreeMap<String, String>);
impl Links {
    fn new() -> Self {
        Self(BTreeMap::new())
    }
    fn link(&mut self, name: &str, id: &str, target: String) -> String {
        if let Some((existing, _)) = self.0.iter().find(|(_, value)| **value == target) {
            return format!("[{}][{existing}]", label(name));
        }
        self.0.insert(id.to_string(), target);
        format!("[{}][{id}]", label(name))
    }
    fn finish(self, output: &mut String) {
        output.push('\n');
        for (id, target) in self.0 {
            output.push_str(&format!("[{id}]: {target}\n"));
        }
    }
}

fn references(output: &mut String, refs: &[Reference], documents: &Catalog, links: &mut Links) {
    if refs.is_empty() {
        return;
    }
    let mut list = Vec::new();
    for reference in refs {
        let page = documents
            .pages
            .iter()
            .find(|p| p.metadata.id == reference.document)
            .expect("validated document reference");
        let fragment = reference
            .anchor
            .as_ref()
            .map_or(String::new(), |a| format!("#{a}"));
        // 两个生成页都位于 docs/reference；正式页面 URL 在导出目录中按 ID 解析。
        let target = format!(
            "../{}{fragment}",
            page.path
                .strip_prefix("docs/")
                .expect("formal document path")
        );
        let id = format!("source-{}-{}", page.metadata.id, links.0.len());
        list.push(links.link(&page.title, &id, target));
    }
    output.push_str(&format!("依据：{}。\n\n", list.join("、")));
}

fn issue_links(output: &mut String, values: &[u32], links: &mut Links) {
    if values.is_empty() {
        return;
    }
    let values: Vec<_> = values
        .iter()
        .map(|n| {
            links.link(
                &format!("#{n}"),
                &format!("issue-{n}"),
                format!("https://github.com/illusion-tech/laneflow/issues/{n}"),
            )
        })
        .collect();
    output.push_str(&format!("登记依据：{}。\n\n", values.join("、")));
}

impl TermCatalog {
    fn choice_links(&self, output: &mut String, choices: &[Choice], links: &mut Links) {
        for choice in choices {
            let (name, anchor) =
                if let Some(term) = self.terms.iter().find(|t| t.record.id == choice.target) {
                    (term.record.name.clone(), format!("term-{}", term.record.id))
                } else {
                    let name = self
                        .name_changes
                        .iter()
                        .find(|t| t.record.id == choice.target)
                        .expect("validated replacement reference");
                    (
                        name.record.names.join(" / "),
                        format!("name-{}", name.record.id),
                    )
                };
            let link = links.link(&name, &anchor, format!("glossary.md#{anchor}"));
            output.push_str(&format!("- {link}：{}\n", choice.condition));
        }
        if !choices.is_empty() {
            output.push('\n');
        }
    }

    fn term(&self, output: &mut String, term: &Term, documents: &Catalog, links: &mut Links) {
        output.push_str(&format!(
            "<a id=\"term-{}\"></a>\n### {}\n\n",
            term.id,
            label(&term.name)
        ));
        if !term.english_aliases.is_empty() {
            output.push_str(&format!(
                "英文辅助名：{}。\n\n",
                term.english_aliases
                    .iter()
                    .map(|s| label(s))
                    .collect::<Vec<_>>()
                    .join("；")
            ));
        }
        if let Some(borrowed) = &term.borrowed {
            output.push_str(&format!(
                "借词；首次出现释义：{}。\n\n",
                borrowed.first_use_gloss
            ));
        }
        output.push_str(&format!("{}\n\n", term.definition));
        if !term.symbols.is_empty() {
            output.push_str(&format!(
                "精确标识符：{}。\n\n",
                term.symbols
                    .iter()
                    .map(|s| format!("{}（{}）", code(&s.value), s.kind.label()))
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        for short in &term.short_forms {
            output.push_str(&format!(
                "允许简称“{}”：{}\n\n",
                label(&short.name),
                short.scope
            ));
        }
        for note in &term.scope_notes {
            output.push_str(&format!("{note}\n\n"));
        }
        if let Some(retirement) = &term.retirement {
            output.push_str(&format!("停止采用：{}\n\n", retirement.reason));
            self.choice_links(output, &retirement.replacements, links);
        }
        if !term.related.is_empty() {
            let related: Vec<_> = term
                .related
                .iter()
                .map(|id| {
                    let target = self
                        .terms
                        .iter()
                        .find(|t| t.record.id == *id)
                        .expect("validated related term");
                    links.link(
                        &target.record.name,
                        &format!("term-{id}"),
                        format!("glossary.md#term-{id}"),
                    )
                })
                .collect();
            output.push_str(&format!("相关概念：{}。\n\n", related.join("、")));
        }
        references(output, &term.references, documents, links);
    }

    pub(super) fn glossary(&self, documents: &Catalog) -> String {
        let mut output = String::new();
        let mut links = Links::new();
        output.push_str("按领域浏览：\n\n");
        for domain in &self.domains {
            output.push_str(&format!(
                "- {}\n",
                links.link(
                    &domain.name,
                    &format!("domain-{}", domain.id),
                    format!("#domain-{}", domain.id)
                )
            ));
        }
        for (name, id) in [
            ("停止采用的概念", "retired-concepts"),
            ("旧称查询", "former-names"),
        ] {
            output.push_str(&format!("- {}\n", links.link(name, id, format!("#{id}"))));
        }
        output.push('\n');
        for domain in &self.domains {
            output.push_str(&format!(
                "<a id=\"domain-{}\"></a>\n## {}\n\n",
                domain.id,
                label(&domain.name)
            ));
            for term in self
                .terms
                .iter()
                .filter(|t| t.record.domains[0] == domain.id && t.record.retirement.is_none())
            {
                self.term(&mut output, &term.record, documents, &mut links);
            }
            let secondary: Vec<_> = self
                .terms
                .iter()
                .filter(|t| {
                    t.record.domains[0] != domain.id
                        && t.record.domains.contains(&domain.id)
                        && t.record.retirement.is_none()
                })
                .collect();
            if !secondary.is_empty() {
                output.push_str("本领域还使用以下概念：\n\n");
                for term in secondary {
                    output.push_str(&format!(
                        "- {}\n",
                        links.link(
                            &term.record.name,
                            &format!("term-{}", term.record.id),
                            format!("glossary.md#term-{}", term.record.id)
                        )
                    ));
                }
                output.push('\n');
            }
        }
        output.push_str("<a id=\"retired-concepts\"></a>\n## 停止采用的概念\n\n本节用于识别旧文档中的概念，不作为现行 API、格式或能力说明。\n\n");
        for term in self.terms.iter().filter(|t| t.record.retirement.is_some()) {
            self.term(&mut output, &term.record, documents, &mut links);
        }
        output.push_str("<a id=\"former-names\"></a>\n## 旧称查询\n\n按下面的适用范围和例外选择规范名；条件化替换不能退化为全文字符串替换。\n\n");
        for located in &self.name_changes {
            let change = &located.record;
            output.push_str(&format!(
                "<a id=\"name-{}\"></a>\n### {}\n\n{}\n\n",
                change.id,
                label(&change.names.join(" / ")),
                change.scope
            ));
            self.choice_links(&mut output, &change.replacements, &mut links);
            if change.include_compounds {
                output.push_str("适用范围包含同义的派生复合词。\n\n");
            }
            for exception in &change.exceptions {
                output.push_str(&format!("例外：{exception}\n\n"));
            }
            issue_links(&mut output, &change.issues, &mut links);
        }
        links.finish(&mut output);
        output
    }

    pub(super) fn wording(&self, documents: &Catalog) -> String {
        let mut output = String::new();
        let mut links = Links::new();
        for located in &self.wording_rules {
            let rule = &located.record;
            output.push_str(&format!(
                "<a id=\"wording-{}\"></a>\n## {}\n\n适用语境：{}\n\n{}\n\n",
                rule.id,
                label(&rule.expressions.join(" / ")),
                rule.scope,
                rule.guidance
            ));
            self.choice_links(&mut output, &rule.alternatives, &mut links);
            for exception in &rule.exceptions {
                output.push_str(&format!("例外与边界：{exception}\n\n"));
            }
            references(&mut output, &rule.references, documents, &mut links);
            issue_links(&mut output, &rule.issues, &mut links);
        }
        links.finish(&mut output);
        output
    }
}
