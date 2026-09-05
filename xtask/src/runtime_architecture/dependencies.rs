//! Cargo 提供节点身份与已解析的边；本仓库声明补充未启用的 optional 边。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use super::{ADAPTER, FORMAT, RUNTIME, SPATIAL, SourceInputs, WIRE};

#[derive(Deserialize)]
pub(super) struct Metadata {
    workspace_members: BTreeSet<String>,
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<Target>,
    dependencies: Vec<Declaration>,
}

#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
    src_path: PathBuf,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Dev,
    Build,
}

#[derive(Deserialize)]
struct Declaration {
    name: String,
    kind: Option<Kind>,
    rename: Option<String>,
    path: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Edge>,
}

#[derive(Deserialize)]
struct Edge {
    pkg: String,
    dep_kinds: Vec<EdgeKind>,
}

#[derive(Deserialize)]
struct EdgeKind {
    kind: Option<Kind>,
}

pub(super) fn load(manifest: &Path, all_features: bool) -> Result<Metadata, String> {
    let mut command = Command::new("cargo");
    command
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .current_dir(manifest.parent().ok_or("manifest 缺少目录")?);
    if all_features {
        command.arg("--all-features");
    }
    let output = command
        .output()
        .map_err(|error| format!("无法读取 Cargo 解析图: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Cargo 解析图读取失败 (all_features={all_features}): {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    // resolve=null（如误用 --no-deps）或缺失字段都不能被当作空图。
    serde_json::from_slice(&output.stdout).map_err(|error| format!("Cargo 解析图不完整: {error}"))
}

impl Metadata {
    fn workspace_package(&self, name: &str) -> Result<&Package, String> {
        self.packages
            .iter()
            .find(|package| package.name == name && self.workspace_members.contains(&package.id))
            .ok_or_else(|| format!("架构依赖图缺少必需 workspace 包 {name}"))
    }

    pub(super) fn source_inputs(&self) -> Result<SourceInputs, String> {
        let runtime = self.workspace_package(RUNTIME)?;
        let libraries: Vec<_> = runtime
            .targets
            .iter()
            .filter(|target| target.kind.iter().any(|kind| kind == "lib"))
            .collect();
        let [library] = libraries.as_slice() else {
            return Err("Runtime 必须具有唯一 Cargo 库入口".into());
        };
        let mut inputs = SourceInputs {
            entry: library.src_path.clone(),
            package_root: runtime
                .manifest_path
                .parent()
                .ok_or("Runtime manifest 缺少目录")?
                .to_path_buf(),
            externals: BTreeSet::from(["std".into(), "core".into(), "alloc".into()]),
            formats: BTreeSet::from([
                "laneflow_format".into(),
                "laneflow_runtime_snapshot_wire".into(),
            ]),
        };
        for dependency in &runtime.dependencies {
            if dependency.kind == Some(Kind::Dev) {
                continue;
            }
            let alias = dependency
                .rename
                .as_deref()
                .unwrap_or(&dependency.name)
                .replace('-', "_");
            inputs.externals.insert(alias.clone());
            if matches!(dependency.name.as_str(), FORMAT | WIRE) {
                inputs.formats.insert(alias);
            }
        }
        inputs.externals.extend(inputs.formats.iter().cloned());
        Ok(inputs)
    }

    pub(super) fn check(&self) -> Result<(), String> {
        for required in [RUNTIME, SPATIAL, ADAPTER] {
            self.workspace_package(required)?;
        }
        let packages: BTreeMap<_, _> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let nodes: BTreeMap<_, _> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut workspace_paths = BTreeMap::new();
        for package in &self.packages {
            if self.workspace_members.contains(&package.id) {
                let directory = package
                    .manifest_path
                    .parent()
                    .ok_or("workspace manifest 缺少目录")?;
                workspace_paths.insert(directory.to_path_buf(), package.id.as_str());
            }
        }
        for source in [RUNTIME, SPATIAL] {
            let initial = self.workspace_package(source)?;
            let mut pending = vec![(initial.id.as_str(), vec![initial.name.clone()])];
            let mut seen = BTreeSet::new();
            while let Some((id, chain)) = pending.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let package = packages
                    .get(id)
                    .ok_or_else(|| format!("已解析包缺失: {id}"))?;
                let node = nodes
                    .get(id)
                    .ok_or_else(|| format!("已解析节点缺失: {id}"))?;
                let mut targets = BTreeSet::new();
                for edge in &node.deps {
                    if edge.dep_kinds.is_empty() {
                        return Err(format!("依赖边缺少 kind: {id} -> {}", edge.pkg));
                    }
                    if edge
                        .dep_kinds
                        .iter()
                        .any(|kind| kind.kind != Some(Kind::Dev))
                    {
                        targets.insert(edge.pkg.as_str());
                    }
                }
                // workspace 声明的禁止依赖即使 feature 未启用也应被拒绝。
                // 外部包只沿解析图遍历，不展开它们未启用的第三方 feature。
                if self.workspace_members.contains(id) {
                    for declaration in &package.dependencies {
                        if declaration.kind == Some(Kind::Dev) {
                            continue;
                        }
                        check_target(source, &declaration.name, &chain)?;
                        if let Some(target) = declaration
                            .path
                            .as_ref()
                            .and_then(|path| workspace_paths.get(path))
                        {
                            targets.insert(*target);
                        }
                    }
                }
                for target in targets {
                    let dependency = packages
                        .get(target)
                        .ok_or_else(|| format!("已解析包缺失: {target}"))?;
                    check_target(source, &dependency.name, &chain)?;
                    let mut next = chain.clone();
                    next.push(dependency.name.clone());
                    pending.push((target, next));
                }
            }
        }
        Ok(())
    }
}

fn check_target(source: &str, target: &str, chain: &[String]) -> Result<(), String> {
    if target == ADAPTER
        || target == "laneflow-compiler"
        || target == "bevy"
        || target.starts_with("bevy_")
        || (source == RUNTIME && target == SPATIAL)
        || (source == SPATIAL && target == RUNTIME)
    {
        return Err(format!(
            "生产依赖方向违规: {} -> {target}",
            chain.join(" -> ")
        ));
    }
    Ok(())
}
