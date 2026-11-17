use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema_version: u32,
    pub files: Vec<String>,
    pub domains: Vec<Domain>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Domain {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub schema_version: u32,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub name_changes: Vec<NameChange>,
    #[serde(default)]
    pub wording_rules: Vec<WordingRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Term {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub english_aliases: Vec<String>,
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub definition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borrowed: Option<Borrowed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub short_forms: Vec<ShortForm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<Symbol>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retirement: Option<Retirement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Borrowed {
    pub language: String,
    pub first_use_gloss: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShortForm {
    pub name: String,
    pub scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Symbol {
    pub value: String,
    pub kind: SymbolKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum SymbolKind {
    Type,
    Crate,
    Field,
    Function,
    Constant,
    Acronym,
    Unit,
    Notation,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "类型",
            Self::Crate => "包",
            Self::Field => "字段",
            Self::Function => "函数",
            Self::Constant => "常量",
            Self::Acronym => "缩写",
            Self::Unit => "单位",
            Self::Notation => "记号",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub document: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Retirement {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replacements: Vec<Choice>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Choice {
    pub target: String,
    pub condition: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NameChange {
    pub id: String,
    pub names: Vec<String>,
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub scope: String,
    pub replacements: Vec<Choice>,
    pub include_compounds: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WordingRule {
    pub id: String,
    pub expressions: Vec<String>,
    pub scope: String,
    pub guidance: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<u32>,
}

#[derive(Debug, Serialize)]
pub(super) struct Located<T> {
    #[serde(flatten)]
    pub record: T,
    pub source: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TermCatalog {
    pub(super) schema_version: u32,
    pub(super) domains: Vec<Domain>,
    pub(super) terms: Vec<Located<Term>>,
    pub(super) name_changes: Vec<Located<NameChange>>,
    pub(super) wording_rules: Vec<Located<WordingRule>>,
}
