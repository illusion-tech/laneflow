use serde::Deserialize;

use super::{EXCLUDED_PACKAGES, valid_package};
use crate::ci_plan::{MetadataDoc, cargo_metadata};

pub(super) const PACKAGE: &str = "laneflow-synth-harness";

#[derive(Deserialize)]
struct PackageTargets {
    name: String,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    test: bool,
    #[serde(default, rename = "required-features")]
    required_features: Vec<String>,
}

impl Target {
    fn default_test(&self) -> bool {
        self.test && self.required_features.is_empty()
    }

    fn selector(&self) -> Option<(&'static str, Option<&str>)> {
        match self.kind.as_slice() {
            [k] if k == "bin" => Some(("--bin", Some(&self.name))),
            [k] if k == "test" => Some(("--test", Some(&self.name))),
            kinds
                if kinds.iter().any(|k| {
                    matches!(
                        k.as_str(),
                        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
                    )
                }) =>
            {
                Some(("--lib", None))
            }
            _ => None,
        }
    }
}

fn select(metadata: &str) -> Result<Vec<String>, String> {
    let doc: MetadataDoc<PackageTargets> =
        serde_json::from_str(metadata).map_err(|err| format!("无法解析 Cargo 目标：{err}"))?;
    let synth = doc
        .packages
        .iter()
        .find(|p| p.name == PACKAGE)
        .ok_or_else(|| format!("缺少 {PACKAGE} 的 Cargo 目标"))?;
    let mut selectors = Vec::new();
    for target in &synth.targets {
        let selector = target.selector().filter(|(flag, _)| {
            (*flag != "--lib" || target.kind == ["lib"])
                && target.default_test()
                && valid_package(&target.name)
        });
        selectors
            .push(selector.ok_or_else(|| format!("Synth 目标 {} 需要默认构建规则", target.name))?);
    }
    if !selectors.contains(&("--lib", None)) {
        return Err("Synth 缺少已核对的普通库测试目标".to_owned());
    }

    // Cargo 的 --lib 和具名目标在整个所选 workspace 匹配。显式点名可能覆盖
    // test = false 或要求未启用的 feature，同名目标也须遵循默认构建范围。
    for pkg in &doc.packages {
        if EXCLUDED_PACKAGES.contains(&pkg.name.as_str()) {
            continue;
        }
        for target in &pkg.targets {
            if !target.default_test() && target.selector().is_some_and(|s| selectors.contains(&s)) {
                return Err(format!(
                    "{} 的目标 {} 需要默认构建规则",
                    pkg.name, target.name
                ));
            }
        }
    }
    let mut args = Vec::new();
    for (flag, name) in selectors {
        args.push(flag.to_owned());
        args.extend(name.map(str::to_owned));
    }
    Ok(args)
}

pub(super) fn cargo_targets(scope: &str) -> Vec<String> {
    if scope != PACKAGE {
        return Vec::new();
    }
    cargo_metadata()
        .and_then(|metadata| select(&metadata))
        .unwrap_or_else(|reason| {
            eprintln!("保留完整 workspace 构建：{reason}");
            Vec::new()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn target(name: &str, kind: &str) -> Value {
        json!({"name": name, "kind": [kind], "test": true})
    }

    fn metadata(synth: &[Value], other: Value) -> String {
        json!({"workspace_root": ".", "packages": [
            {"name": PACKAGE, "targets": synth},
            other,
        ]})
        .to_string()
    }

    #[test]
    fn discovers_new_synth_targets_and_keeps_other_scopes_on_the_default_build() {
        let targets = [
            target("library", "lib"),
            target("tool", "bin"),
            target("new_suite", "test"),
        ];
        let other = json!({"name": "other", "targets": [target("new_suite", "test")]});
        assert_eq!(
            select(&metadata(&targets, other)).unwrap(),
            ["--lib", "--bin", "tool", "--test", "new_suite"]
        );
        for scope in ["*", "xtask", "laneflow-synth-harness xtask"] {
            assert!(cargo_targets(scope).is_empty());
        }
    }

    #[test]
    fn defaults_to_the_original_build_for_unsupported_or_conditional_targets() {
        let other = json!({"name": "other", "targets": []});
        let mut disabled = target("off", "bin");
        disabled["test"] = false.into();
        let mut gated = target("gated", "test");
        gated["required-features"] = json!(["opt-in"]);
        for t in [
            target("demo", "example"),
            target("suite*", "test"),
            disabled,
            gated,
        ] {
            assert!(select(&metadata(&[t], other.clone())).is_err());
        }
        assert!(select(&metadata(&[], other.clone())).is_err());
        assert!(select(&metadata(&[target("bin_only", "bin")], other)).is_err());
        assert!(select("{}").is_err());
    }

    #[test]
    fn considers_collisions_only_in_packages_selected_by_cargo() {
        let synth = [target("library", "lib"), target("cli", "test")];
        let mut gated = target("cli", "test");
        gated["required-features"] = json!(["opt-in"]);
        let mut disabled_lib = target("other_library", "proc-macro");
        disabled_lib["test"] = false.into();
        for t in [gated, disabled_lib] {
            let other = json!({"name":"other", "targets":[t]});
            assert!(select(&metadata(&synth, other.clone())).is_err());
            let mut excluded = other;
            excluded["name"] = EXCLUDED_PACKAGES[0].into();
            assert!(select(&metadata(&synth, excluded)).is_ok());
        }
    }
}
