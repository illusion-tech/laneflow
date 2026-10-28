//! 执行框架按产品与入口形态显式登记；别名和模糊名称只用于诊断（#1128）。

use std::collections::BTreeSet;
use std::sync::LazyLock;

const REGISTRY_PATH: &str = "xtask/data/harness-registry.json";
const REGISTRY_JSON: &str = include_str!("../data/harness-registry.json");
static REGISTRY: LazyLock<Result<HarnessRegistry, String>> =
    LazyLock::new(|| HarnessRegistry::parse(REGISTRY_JSON));

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    entries: Vec<HarnessEntry>,
    hints: Vec<HarnessHint>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessEntry {
    id: String,
    display_name: String,
    sources: Vec<HarnessSource>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessSource {
    url: String,
    checked_at: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessHint {
    input: String,
    candidates: Vec<String>,
}

struct HarnessRegistry {
    ids: BTreeSet<String>,
    hints: Vec<HarnessHint>,
}

pub(crate) fn validate_harness_id(value: &str) -> Result<(), String> {
    REGISTRY
        .as_ref()
        .map_err(|error| format!("执行框架目录 `{REGISTRY_PATH}` 无效：{error}"))?
        .validate(value)
}

impl HarnessRegistry {
    fn parse(json: &str) -> Result<Self, String> {
        let file: RegistryFile =
            serde_json::from_str(json).map_err(|error| format!("无法解析执行框架目录：{error}"))?;
        let mut ids = BTreeSet::new();
        for entry in file.entries {
            if !valid_id(&entry.id) || !ids.insert(entry.id.clone()) {
                return Err(format!("规范执行框架 ID `{}` 格式无效或重复", entry.id));
            }
            if !clean_text(&entry.display_name)
                || entry.sources.is_empty()
                || entry.sources.iter().any(|source| !valid_source(source))
            {
                return Err(format!(
                    "执行框架 `{}` 必须提供展示名、HTTPS 来源和 YYYY-MM-DD 核实日期",
                    entry.id
                ));
            }
        }
        if ids.is_empty() {
            return Err("执行框架目录不能为空".to_string());
        }

        let mut inputs = BTreeSet::new();
        for hint in &file.hints {
            let mut candidates = BTreeSet::new();
            if !clean_text(&hint.input)
                || !inputs.insert(hint.input.to_ascii_lowercase())
                || ids.iter().any(|id| id.eq_ignore_ascii_case(&hint.input))
                || hint.candidates.is_empty()
                || hint
                    .candidates
                    .iter()
                    .any(|id| !ids.contains(id) || !candidates.insert(id))
            {
                return Err(format!("执行框架提示 `{}` 的登记无效或冲突", hint.input));
            }
        }

        Ok(Self {
            ids,
            hints: file.hints,
        })
    }

    fn validate(&self, value: &str) -> Result<(), String> {
        if self.ids.contains(value) {
            return Ok(());
        }

        // 大小写与展示名只影响提示；绝不扩张已登记的产品与入口组合。
        let mut suggestions = BTreeSet::new();
        for id in &self.ids {
            if id.eq_ignore_ascii_case(value) {
                suggestions.insert(id.as_str());
            }
        }
        for hint in &self.hints {
            if hint.input.eq_ignore_ascii_case(value) {
                suggestions.extend(hint.candidates.iter().map(String::as_str));
            }
        }
        let mut error = format!(
            "`Harness: {value}` 未登记为规范 ID；只接受已登记的 `<发布者>/<产品>/<入口形态>`。请核对实际会话入口，或先在 `{REGISTRY_PATH}` 补充有来源的登记；参见 docs/reference/commit-convention.md 第 6.2 节"
        );
        if !suggestions.is_empty() {
            error.push_str("\n请按实际入口选择（候选不会自动接受或改写）：");
            for id in suggestions {
                error.push_str(&format!("\n  Harness: {id}"));
            }
        }
        Err(error)
    }
}

fn clean_text(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn valid_id(id: &str) -> bool {
    let mut parts = id.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(publisher), Some(product), Some(surface), None)
            if valid_slug(publisher)
                && valid_slug(product)
                && matches!(surface, "cli" | "web" | "desktop" | "ide-extension" | "ide")
    )
}

fn valid_slug(value: &str) -> bool {
    !matches!(value, "unknown" | "other")
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_source(source: &HarnessSource) -> bool {
    source.url.strip_prefix("https://").is_some_and(|rest| {
        rest.split('/').next().is_some_and(|host| !host.is_empty())
            && !rest.chars().any(|c| c.is_whitespace() || c.is_control())
    }) && valid_date(&source.checked_at)
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| i != 4 && i != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or_default();
    let month = value[5..7].parse::<u32>().unwrap_or_default();
    let day = value[8..].parse::<u32>().unwrap_or_default();
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "entries": [
                {
                    "id": "vendor/agent/cli", "display_name": "Agent CLI",
                    "sources": [{"url": "https://example.com/cli", "checked_at": "2026-10-28"}]
                },
                {
                    "id": "vendor/agent/desktop", "display_name": "Agent Desktop",
                    "sources": [{"url": "https://example.com/desktop", "checked_at": "2026-10-28"}]
                }
            ],
            "hints": [
                {"input": "Agent", "candidates": ["vendor/agent/cli", "vendor/agent/desktop"]},
                {"input": "Agent CLI", "candidates": ["vendor/agent/cli"]}
            ]
        })
    }

    #[test]
    fn bundled_registry_accepts_registered_surfaces_individually() {
        let registry = HarnessRegistry::parse(REGISTRY_JSON).expect("valid bundled registry");
        for surface in ["cli", "web", "desktop", "ide-extension"] {
            assert!(
                registry
                    .validate(&format!("openai/codex/{surface}"))
                    .is_ok()
            );
        }
        assert!(registry.validate("anysphere/cursor/ide").is_ok());
        assert!(registry.validate("anysphere/cursor/cli").is_ok());
        assert!(registry.validate("anysphere/cursor/ide-extension").is_err());
        assert!(registry.validate("deepseek/harness/desktop").is_ok());
        assert!(registry.validate("deepseek/harness/cli").is_err());
        for id in &registry.ids {
            assert!(registry.validate(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn aliases_and_case_variants_suggest_but_never_pass() {
        let registry = HarnessRegistry::parse(&fixture().to_string()).unwrap();
        for value in ["Agent CLI", "AGENT CLI", "Vendor/Agent/CLI"] {
            let error = registry.validate(value).expect_err("alias rejected");
            assert!(error.contains("Harness: vendor/agent/cli"), "{error}");
            assert!(!error.contains("Harness: vendor/agent/desktop"), "{error}");
        }
        assert!(registry.validate("vendor/agent/cli").is_ok());
    }

    #[test]
    fn ambiguous_names_list_all_candidates_without_choosing_a_surface() {
        let registry = HarnessRegistry::parse(&fixture().to_string()).unwrap();
        let error = registry.validate("Agent").unwrap_err();
        assert!(error.contains("Harness: vendor/agent/cli"), "{error}");
        assert!(error.contains("Harness: vendor/agent/desktop"), "{error}");
    }

    #[test]
    fn unregistered_combinations_and_placeholders_require_registration() {
        let registry = HarnessRegistry::parse(&fixture().to_string()).unwrap();
        for value in [
            "vendor/agent/web",
            "vendor/agent",
            "other/agent/cli",
            "unknown",
            "other",
            "New Agent",
            " vendor/agent/cli",
            "vendor/agent/cli ",
        ] {
            let error = registry.validate(value).expect_err("unregistered rejected");
            assert!(error.contains(REGISTRY_PATH), "{error}");
            assert!(!error.contains("\n  Harness:"), "{error}");
        }
    }

    #[test]
    fn malformed_ids_cannot_be_registered() {
        for id in [
            "agent",
            "vendor/agent",
            "vendor/agent/cli/extra",
            "/agent/cli",
            "vendor//cli",
            "vendor/agent/",
            "Vendor/agent/cli",
            "vendor/Agent/cli",
            "vendor/agent/CLI",
            "vendor/agent/tui",
            "vendor/agent/cloud",
            "vendor/agent/api",
            "vendor/agent/other",
            "vendor/agent/unknown",
            "unknown/agent/cli",
            "vendor/other/cli",
            "vendor/agent_/cli",
            "vendor/agent.name/cli",
            "vendor/-agent/cli",
            "vendor/agent-/cli",
            "vendor/agent--name/cli",
            "vendor/age nt/cli",
            "vendor/agent/cli\n",
            "厂商/agent/cli",
        ] {
            let mut file = fixture();
            file["entries"][0]["id"] = id.into();
            file["hints"] = serde_json::json!([]);
            assert!(HarnessRegistry::parse(&file.to_string()).is_err(), "{id:?}");
        }
    }

    #[test]
    fn malformed_empty_and_duplicate_registries_fail_closed() {
        for json in ["invalid json", "{}", r#"{"entries":[],"hints":[]}"#] {
            assert!(HarnessRegistry::parse(json).is_err());
        }
        let mut file = fixture();
        let duplicate = file["entries"][0].clone();
        file["entries"].as_array_mut().unwrap().push(duplicate);
        assert!(HarnessRegistry::parse(&file.to_string()).is_err());
        let mut file = fixture();
        file["unexpected"] = true.into();
        assert!(HarnessRegistry::parse(&file.to_string()).is_err());
    }

    #[test]
    fn entries_require_display_names_and_dated_sources() {
        for value in ["", " ", "Name\n"] {
            let mut file = fixture();
            file["entries"][0]["display_name"] = value.into();
            assert!(HarnessRegistry::parse(&file.to_string()).is_err());
        }
        for sources in [
            serde_json::json!([]),
            serde_json::json!([{"url": "", "checked_at": "2026-10-28"}]),
            serde_json::json!([{"url": "https://", "checked_at": "2026-10-28"}]),
            serde_json::json!([{"url": "https://example.com/bad url", "checked_at": "2026-10-28"}]),
            serde_json::json!([{"url": "https://example.com", "checked_at": ""}]),
            serde_json::json!([{"url": "https://example.com"}]),
            serde_json::json!([{"url": "https://example.com", "checked_at": "2026-02-29"}]),
        ] {
            let mut file = fixture();
            file["entries"][0]["sources"] = sources;
            assert!(HarnessRegistry::parse(&file.to_string()).is_err());
        }
    }

    #[test]
    fn checked_dates_are_real_calendar_dates() {
        for value in ["2026-10-28", "2024-02-29", "2000-02-29"] {
            assert!(valid_date(value), "{value}");
        }
        for value in [
            "2026-2-28",
            "2026-13-01",
            "2026-04-31",
            "2026-01-00",
            "1900-02-29",
            "0000-01-01",
            "二〇二六-10-28",
        ] {
            assert!(!valid_date(value), "{value}");
        }
    }

    #[test]
    fn conflicting_empty_and_unknown_hint_targets_are_rejected() {
        for hint in [
            serde_json::json!({"input": "", "candidates": ["vendor/agent/cli"]}),
            serde_json::json!({"input": "Agent\n", "candidates": ["vendor/agent/cli"]}),
            serde_json::json!({"input": "Agent", "candidates": []}),
            serde_json::json!({"input": "Agent", "candidates": ["vendor/agent/web"]}),
            serde_json::json!({"input": "Agent", "candidates": ["vendor/agent/cli", "vendor/agent/cli"]}),
            serde_json::json!({"input": "VENDOR/AGENT/CLI", "candidates": ["vendor/agent/desktop"]}),
        ] {
            let mut file = fixture();
            file["hints"] = serde_json::json!([hint]);
            assert!(HarnessRegistry::parse(&file.to_string()).is_err());
        }
        let mut file = fixture();
        file["hints"][1]["input"] = "AGENT".into();
        assert!(HarnessRegistry::parse(&file.to_string()).is_err());
    }
}
