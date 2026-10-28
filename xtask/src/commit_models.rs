//! 提交模型目录只接受已登记的精确 ID；别名只用于诊断提示（#1125）。

use std::collections::BTreeSet;
use std::sync::LazyLock;

const REGISTRY_PATH: &str = "xtask/data/model-registry.json";
const REGISTRY_JSON: &str = include_str!("../data/model-registry.json");
static REGISTRY: LazyLock<Result<ModelRegistry, String>> =
    LazyLock::new(|| ModelRegistry::parse(REGISTRY_JSON));

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    source: String,
    retrieved_at: String,
    canonical_ids: Vec<String>,
    local_models: Vec<LocalModel>,
    aliases: Vec<ModelAlias>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalModel {
    id: String,
    source: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelAlias {
    alias: String,
    canonical_id: String,
    source: String,
}

struct ModelRegistry {
    canonical_ids: BTreeSet<String>,
    aliases: Vec<ModelAlias>,
}

pub(crate) fn validate_model_id(value: &str) -> Result<(), String> {
    REGISTRY
        .as_ref()
        .map_err(|error| format!("模型目录 `{REGISTRY_PATH}` 无效：{error}"))?
        .validate(value)
}

impl ModelRegistry {
    fn parse(json: &str) -> Result<Self, String> {
        let file: RegistryFile =
            serde_json::from_str(json).map_err(|error| format!("无法解析模型目录：{error}"))?;
        if file.source.trim().is_empty() || file.retrieved_at.trim().is_empty() {
            return Err("目录必须记录上游来源和获取时间".to_string());
        }

        let mut canonical_ids = BTreeSet::new();
        for id in file.canonical_ids {
            register_id(&mut canonical_ids, id)?;
        }
        for model in file.local_models {
            if model.source.trim().is_empty() {
                return Err(format!("本地模型 `{}` 缺少来源", model.id));
            }
            register_id(&mut canonical_ids, model.id)?;
        }
        if canonical_ids.is_empty() {
            return Err("模型目录不能为空".to_string());
        }

        let mut aliases = BTreeSet::new();
        for entry in &file.aliases {
            if entry.alias.trim().is_empty()
                || entry.alias.trim() != entry.alias
                || entry.alias.chars().any(char::is_control)
                || entry.source.trim().is_empty()
                || !aliases.insert(entry.alias.to_ascii_lowercase())
                || canonical_ids.contains(&entry.alias)
                || !canonical_ids.contains(&entry.canonical_id)
            {
                return Err(format!("模型别名 `{}` 的登记无效或冲突", entry.alias));
            }
        }

        Ok(Self {
            canonical_ids,
            aliases: file.aliases,
        })
    }

    fn validate(&self, value: &str) -> Result<(), String> {
        if self.canonical_ids.contains(value) {
            return Ok(());
        }

        // 提示可忽略大小写和缺失的实验室前缀，但不得据此接受或静默改写 ID。
        let mut suggestions = BTreeSet::new();
        for id in &self.canonical_ids {
            if id.eq_ignore_ascii_case(value)
                || id
                    .split_once('/')
                    .is_some_and(|(_, name)| name.eq_ignore_ascii_case(value))
            {
                suggestions.insert(id.as_str());
            }
        }
        for entry in &self.aliases {
            if entry.alias.eq_ignore_ascii_case(value) {
                suggestions.insert(entry.canonical_id.as_str());
            }
        }

        if suggestions.len() == 1 {
            let canonical = suggestions.first().expect("one suggestion");
            return Err(format!(
                "`Model: {value}` 不是规范 ID；请核对实际模型并使用 `Model: {canonical}`"
            ));
        }
        Err(format!(
            "`Model: {value}` 未登记为规范 ID；请核对实际模型，或先在 `{REGISTRY_PATH}` 更新目录/补充有来源的本地登记；参见 docs/reference/commit-convention.md 第 6.1 节"
        ))
    }
}

fn register_id(ids: &mut BTreeSet<String>, id: String) -> Result<(), String> {
    let valid = id
        .split_once('/')
        .is_some_and(|(lab, name)| !lab.is_empty() && !name.is_empty() && !name.contains('/'))
        && !id.chars().any(|c| c.is_whitespace() || c.is_control());
    if !valid || ids.iter().any(|known| known.eq_ignore_ascii_case(&id)) {
        return Err(format!("规范模型 ID `{id}` 格式无效或重复"));
    }
    ids.insert(id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "source": "https://models.dev/models.json?type=all",
            "retrieved_at": "2026-10-28T08:28:23Z",
            "canonical_ids": ["lab/model-1", "lab/model-1-fast"],
            "local_models": [],
            "aliases": [{
                "alias": "old-model",
                "canonical_id": "lab/model-1",
                "source": "https://example.com/models"
            }]
        })
    }

    #[test]
    fn bundled_registry_is_valid_and_preserves_variants() {
        let registry = ModelRegistry::parse(REGISTRY_JSON).expect("valid bundled registry");
        for id in [
            "anthropic/claude-opus-5-5",
            "openai/gpt-6.1-sol",
            "xiaomi/mimo-v2.6-pro",
            "xiaomi/mimo-v2.6-pro-ultraspeed",
        ] {
            assert!(registry.validate(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn aliases_and_case_variants_only_suggest_exact_ids() {
        let registry = ModelRegistry::parse(&fixture().to_string()).unwrap();
        for value in ["old-model", "OLD-MODEL", "LAB/MODEL-1", "model-1"] {
            let error = registry.validate(value).expect_err("alias rejected");
            assert!(error.contains("Model: lab/model-1"), "{error}");
        }
        assert!(registry.validate("lab/model-1").is_ok());
        assert!(registry.validate("lab/model-1-fast").is_ok());
    }

    #[test]
    fn unknown_and_ambiguous_ids_require_registration() {
        let mut file = fixture();
        file["canonical_ids"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("other/model-1"));
        let registry = ModelRegistry::parse(&file.to_string()).unwrap();
        for value in ["model-1", "GPT-6", "lab/new-model", " lab/model-1"] {
            let error = registry.validate(value).expect_err("unknown rejected");
            assert!(error.contains(REGISTRY_PATH), "{error}");
            assert!(!error.contains("并使用"), "ambiguous suggestion: {error}");
        }
    }

    #[test]
    fn local_registration_requires_source_and_no_duplicate() {
        let mut file = fixture();
        file["local_models"] = serde_json::json!([{
            "id": "local/new-model", "source": "https://example.com/new-model"
        }]);
        let registry = ModelRegistry::parse(&file.to_string()).unwrap();
        assert!(registry.validate("local/new-model").is_ok());
        file["local_models"][0]["source"] = serde_json::json!("");
        assert!(ModelRegistry::parse(&file.to_string()).is_err());
        file["local_models"][0]["source"] = serde_json::json!("https://example.com/new-model");
        file["local_models"][0]["id"] = serde_json::json!("lab/model-1");
        assert!(ModelRegistry::parse(&file.to_string()).is_err());
    }

    #[test]
    fn malformed_registry_cannot_be_used() {
        for ids in [
            serde_json::json!([]),
            serde_json::json!(["lab/model-1", "lab/model-1"]),
            serde_json::json!(["lab/model-1", "LAB/MODEL-1"]),
            serde_json::json!(["model-1"]),
            serde_json::json!(["lab/model 1"]),
            serde_json::json!(["/model-1"]),
            serde_json::json!(["lab/"]),
        ] {
            let mut file = fixture();
            file["canonical_ids"] = ids;
            assert!(ModelRegistry::parse(&file.to_string()).is_err());
        }
        assert!(ModelRegistry::parse("{}").is_err());
        assert!(ModelRegistry::parse("invalid json").is_err());
    }

    #[test]
    fn conflicting_aliases_cannot_be_used() {
        let mut file = fixture();
        file["aliases"][0]["canonical_id"] = serde_json::json!("lab/unknown");
        assert!(ModelRegistry::parse(&file.to_string()).is_err());
        let mut file = fixture();
        let mut second = file["aliases"][0].clone();
        second["alias"] = serde_json::json!("OLD-MODEL");
        second["canonical_id"] = serde_json::json!("lab/model-1-fast");
        file["aliases"].as_array_mut().unwrap().push(second);
        assert!(ModelRegistry::parse(&file.to_string()).is_err());
    }
}
