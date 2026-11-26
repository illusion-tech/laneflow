//! 对照 Fuzz 候选范围。选测结论只写入报告。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    FileState, MetadataDoc, REGISTRY_TOML, Registry, UNPARSED_INCLUDE, Workspace,
    append_step_summary, cargo_metadata, changed_files, classify, collect_fuzz_includes,
    event_changed_files, plan_markdown, read_files_list, repo_relative, repo_root,
    resolve_embed_target, walk_sources,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TestTarget {
    package: String,
    name: String,
    #[serde(default)]
    modules: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    targets: Vec<CargoTarget>,
}

#[derive(Deserialize)]
struct CargoTarget {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

fn test_inputs(
    root: &Path,
    metadata: &str,
    registrations: &[TestTarget],
) -> Result<BTreeMap<String, String>, String> {
    let metadata: MetadataDoc<Package> = serde_json::from_str(metadata)
        .map_err(|error| format!("无法解析 Cargo 测试目标：{error}"))?;
    let mut inputs = BTreeMap::new();
    for entry in registrations {
        let label = format!("{}/{}", entry.package, entry.name);
        let target = metadata
            .packages
            .iter()
            .find(|package| package.name == entry.package)
            .and_then(|package| {
                package.targets.iter().find(|target| {
                    target.name == entry.name && target.kind.iter().any(|kind| kind == "test")
                })
            })
            .ok_or_else(|| format!("登记的 Cargo 测试目标 {label} 已不存在"))?;
        let source = std::fs::read_to_string(&target.src_path)
            .map_err(|error| format!("无法读取测试目标 {label}：{error}"))?;
        let parsed = syn::parse_file(&source)
            .map_err(|error| format!("无法解析测试目标 {label}：{error}"))?;
        let mut paths = vec![repo_relative(&metadata.workspace_root, &target.src_path)?];
        for module in &entry.modules {
            let attached = parsed.items.iter().any(|item| {
                let syn::Item::Mod(item) = item else {
                    return false;
                };
                item.content.is_none()
                    && item.attrs.iter().any(|attribute| {
                        if let syn::Meta::NameValue(value) = &attribute.meta
                            && value.path.is_ident("path")
                            && let syn::Expr::Lit(expression) = &value.value
                            && let syn::Lit::Str(path) = &expression.lit
                        {
                            path.value() == *module
                        } else {
                            false
                        }
                    })
            });
            if !attached {
                return Err(format!("测试目标 {label} 的私有模块 {module} 与登记不符"));
            }
            paths.push(resolve_embed_target(
                &metadata.workspace_root,
                &target.src_path,
                module,
            )?);
        }
        for path in paths {
            let canonical = root
                .join(&path)
                .canonicalize()
                .map_err(|error| format!("无法核对已登记的测试源码 {path}：{error}"))?;
            if !canonical.is_file()
                || repo_relative(root, &canonical)? != path
                || metadata
                    .packages
                    .iter()
                    .flat_map(|package| &package.targets)
                    .any(|other| {
                        !other.kind.iter().any(|kind| kind == "test")
                            && repo_relative(&metadata.workspace_root, &other.src_path).as_deref()
                                == Ok(path.as_str())
                    })
            {
                return Err(format!("无法确认 {path} 只用于已登记的测试目标"));
            }
            if inputs.insert(path.clone(), label.clone()).is_some() {
                return Err(format!("Fuzz 对照的测试输入登记重复：{path}"));
            }
        }
    }
    Ok(inputs)
}

fn fuzz_inputs(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut inputs = BTreeMap::new();
    let manifest = std::fs::read_to_string(root.join("fuzz/Cargo.toml"))
        .map_err(|error| format!("无法读取 Fuzz 清单：{error}"))?;
    let manifest: toml::Table =
        toml::from_str(&manifest).map_err(|error| format!("无法解析 Fuzz 清单：{error}"))?;
    let build = manifest
        .get("package")
        .ok_or("Fuzz 清单缺少 package")?
        .get("build");
    if build.is_some_and(|value| value.as_bool() != Some(false))
        || root.join("fuzz/build.rs").exists()
    {
        return Err("Fuzz 的构建脚本输入需核对".to_owned());
    }
    walk_sources(&root.join("fuzz"), &mut |file| {
        let source = std::fs::read_to_string(file)
            .map_err(|error| format!("无法读取 Fuzz 源码 {}：{error}", file.display()))?;
        let parsed = syn::parse_file(&source)
            .map_err(|error| format!("无法解析 Fuzz 源码 {}：{error}", file.display()))?;
        let relative = repo_relative(root, file)?;
        for literal in collect_fuzz_includes(&parsed) {
            if literal == UNPARSED_INCLUDE
                || Path::new(&literal).is_absolute()
                || literal.contains('\\')
            {
                return Err(format!("{relative} 含尚无法核对的文件输入"));
            }
            inputs.insert(
                resolve_embed_target(root, file, &literal)?,
                relative.clone(),
            );
        }
        Ok(())
    })?;
    Ok(inputs)
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scope {
    None,
    Full,
    Fallback,
}

#[derive(Serialize)]
struct Comparison {
    scope: Scope,
    files: BTreeMap<String, String>,
    error: Option<String>,
}

fn compare(
    files: BTreeSet<String>,
    root: &Path,
    metadata: &str,
    registry: &Registry,
) -> Result<Comparison, String> {
    let workspace = Workspace::parse(metadata)?;
    let fuzz_inputs = fuzz_inputs(root)?;
    let test_inputs = test_inputs(root, metadata, &registry.fuzz_comparison_tests)?;
    let mut candidate = Comparison {
        scope: Scope::None,
        files: BTreeMap::new(),
        error: None,
    };
    for path in files {
        let state = classify(&path, &workspace, registry);
        let (scope, reason) = if let Some(reader) = fuzz_inputs.get(&path) {
            (Scope::Full, format!("Fuzz 源码 {reader:?} 直接读取的输入"))
        } else if path.starts_with("fuzz/")
            || path.starts_with("schemas/road-editing/")
            || path.starts_with("schemas/runtime-snapshot/")
        {
            (Scope::Full, "Fuzz 实现、配置或相关 schema".to_owned())
        } else {
            match state {
                FileState::FullTrigger(rule) => (Scope::Full, format!("共享配置或规则 {rule}")),
                FileState::Input { .. } => (Scope::Full, "已登记的跨包或运行时输入".to_owned()),
                FileState::Package(package) => match test_inputs.get(&path) {
                    Some(target) => (Scope::None, format!("已核对的测试目标 {target} 的源码")),
                    None => (Scope::Full, format!("包 {package} 的输入，保留完整矩阵")),
                },
                FileState::NotApplicable(rule)
                    if matches!(
                        rule.as_str(),
                        "docs/**" | "README.md" | "CONTRIBUTING.md" | "AGENTS.md"
                    ) =>
                {
                    (Scope::None, "已登记的独立文档".to_owned())
                }
                _ => (Scope::Fallback, "找不到可排除的输入依据".to_owned()),
            }
        };
        if scope == Scope::Fallback || (scope == Scope::Full && candidate.scope == Scope::None) {
            candidate.scope = scope;
        }
        candidate.files.insert(path, reason);
    }
    Ok(candidate)
}

impl Comparison {
    fn summary(&self) -> String {
        let mut text = format!(
            "{}\n",
            match self.scope {
                Scope::None => "候选为零任务。",
                Scope::Full => "候选为完整 Fuzz 矩阵。",
                Scope::Fallback => "候选回退到完整 Fuzz 矩阵。",
            }
        );
        if let Some(error) = &self.error {
            let _ = writeln!(text, "原因 {error:?}");
        }
        for (path, reason) in &self.files {
            let _ = writeln!(text, "- {path:?} → {reason}");
        }
        text
    }
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let files = match args {
        [] => std::env::var("GITHUB_EVENT_NAME")
            .map_err(|_| "缺少 GITHUB_EVENT_NAME".to_owned())
            .and_then(|event| event_changed_files(&event)),
        [flag, path] if flag == "--files" => read_files_list(path),
        [flag, range] if flag == "--rev-range" => changed_files(range),
        _ => return Err("ci-plan --compare-fuzz 需要事件环境、--files 或 --rev-range".to_owned()),
    };
    let comparison = files
        .and_then(|files| {
            compare(
                files,
                &repo_root()?,
                &cargo_metadata()?,
                &Registry::parse(REGISTRY_TOML)?,
            )
        })
        .unwrap_or_else(|error| Comparison {
            scope: Scope::Fallback,
            files: BTreeMap::new(),
            error: Some(error),
        });
    let summary = comparison.summary();
    let json = serde_json::to_string_pretty(&comparison)
        .map_err(|error| format!("无法写出 Fuzz 对照 JSON：{error}"))?;
    if std::env::var_os("GITHUB_STEP_SUMMARY").is_some() {
        println!("{}", summary.lines().next().unwrap_or_default());
    }
    append_step_summary(&plan_markdown("Fuzz 选测对照", &summary, &json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_inputs_distinguish_private_tests_from_fuzz_inputs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let registry = Registry::parse(REGISTRY_TOML).unwrap();
        let metadata = cargo_metadata().unwrap();
        let only_test = [
            "runtime_profile_evidence.rs",
            "runtime_cpu_sampling.rs",
            "runtime_journal_ab.rs",
        ]
        .map(|name| format!("crates/laneflow-runtime/src/kernel/tests/performance_profile/{name}"));
        let runtime = "crates/laneflow-runtime/src/kernel/world.rs";
        let fixture = "crates/laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca";
        let evaluate = |files| compare(files, &root, &metadata, &registry).unwrap();
        assert_eq!(
            evaluate(only_test.iter().cloned().collect()).scope,
            Scope::None
        );
        let fixture = evaluate(BTreeSet::from([fixture.to_owned()]));
        assert_eq!(fixture.scope, Scope::Full);
        assert!(
            fixture
                .files
                .values()
                .any(|reason| reason.contains("直接读取"))
        );
        assert_eq!(
            evaluate(BTreeSet::from([only_test[0].clone(), runtime.to_owned()])).scope,
            Scope::Full
        );
        for config in [
            "Cargo.lock",
            "fuzz/Cargo.lock",
            ".github/workflows/fuzz.yml",
        ] {
            assert_eq!(
                evaluate(BTreeSet::from([config.to_owned()])).scope,
                Scope::Full
            );
        }
        assert_eq!(
            evaluate(BTreeSet::from(["unregistered.input".to_owned()])).scope,
            Scope::Fallback
        );
    }

    #[test]
    fn fuzz_includes_reject_source_expansion_or_unresolved_input() {
        for source in [
            r#"fn check() { include!("external.rs"); }"#,
            r#"#[cfg_attr(unix, path = "../current.rs")] mod source;"#,
            r#"fn check() { let data = include_bytes!(concat!("x", "/y")); }"#,
        ] {
            let syntax = syn::parse_file(source).unwrap();
            assert!(
                collect_fuzz_includes(&syntax)
                    .iter()
                    .any(|path| path == UNPARSED_INCLUDE),
                "{source}"
            );
        }
    }
}
