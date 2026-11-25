//! 为受影响包选择等价的默认测试构建命令。

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::super::{MetadataDoc, cargo_metadata};

mod graph;

use graph::{Build, Graph, UnitIds};

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    rename: Option<String>,
}

pub(super) struct Selection {
    pub(super) args: Vec<String>,
    pub(super) summary: String,
}

pub(super) fn select(scope: &str, workspace: &[String]) -> Result<Selection, String> {
    let metadata: MetadataDoc<Package> = serde_json::from_str(&cargo_metadata()?)
        .map_err(|error| format!("无法读取 Cargo 包与依赖的名称：{error}"))?;
    let by_name: BTreeMap<_, _> = metadata
        .packages
        .iter()
        .map(|pkg| (pkg.name.as_str(), pkg))
        .collect();
    let packages: BTreeMap<_, _> = scope
        .split(' ')
        .map(|name| {
            by_name
                .get(name)
                .map(|&pkg| (pkg.id.as_str(), pkg))
                .ok_or_else(|| format!("Cargo 元数据缺少包 {name}"))
        })
        .collect::<Result<_, _>>()?;
    let selected: BTreeSet<_> = packages.keys().copied().collect();
    let baseline = Graph::read(workspace)?;
    let mut ids = UnitIds::new();
    let baseline_scope = baseline.scope(&mut ids, &selected)?;
    let mut verify = |graph: &Graph, args| {
        let candidate = graph.scope(&mut ids, &selected)?;
        Ok::<_, String>(candidate.preserves(&baseline_scope).then(|| Selection {
            args,
            summary: format!(
                "Cargo 构建图核对一致，保留所选包的 {} 个目标及其传递依赖。构建单元由 {} 减至 {}。",
                candidate.selected_roots(),
                baseline_scope.units(),
                candidate.units()
            ),
        }))
    };

    let mut args: Vec<_> = scope
        .split(' ')
        .flat_map(|name| ["-p".to_owned(), name.to_owned()])
        .collect();
    if let Ok(direct) = Graph::read(&args) {
        if let Some(selection) = verify(&direct, args.clone())? {
            return Ok(selection);
        }
        let missing = missing_features(&baseline, &direct, &packages);
        if !missing.is_empty() {
            args.extend(["--features".to_owned(), missing.join(",")]);
            if let Ok(repaired) = Graph::read(&args)
                && let Some(selection) = verify(&repaired, args)?
            {
                return Ok(selection);
            }
        }
    }

    if let Some(targets) = test_targets(&baseline, &selected) {
        let mut args = workspace.to_vec();
        args.extend(targets);
        if let Ok(targeted) = Graph::read(&args)
            && let Some(selection) = verify(&targeted, args)?
        {
            return Ok(selection);
        }
    }
    Err("没有候选命令能等价收窄 Cargo 构建图".to_owned())
}

fn feature_key(build: &Build) -> (&str, &str, &str) {
    (&build.package, &build.target.name, &build.mode)
}

fn missing_features(
    baseline: &Graph,
    candidate: &Graph,
    packages: &BTreeMap<&str, &Package>,
) -> Vec<String> {
    let mut original = BTreeMap::<_, BTreeSet<&str>>::new();
    for unit in &baseline.units {
        original
            .entry(feature_key(&unit.build))
            .or_default()
            .extend(unit.build.features.iter().map(String::as_str));
    }
    let mut missing = BTreeSet::new();
    for unit in &candidate.units {
        let Some(package) = packages.get(unit.build.package.as_str()) else {
            continue;
        };
        for edge in &unit.dependencies {
            let dependency = candidate.dependency(edge);
            let Some(features) = original.get(&feature_key(dependency)) else {
                continue;
            };
            let Some(alias) = package
                .dependencies
                .iter()
                .map(|dep| dep.rename.as_deref().unwrap_or(&dep.name))
                .find(|alias| alias.replace('-', "_") == edge.extern_crate_name)
            else {
                continue;
            };
            // 优先恢复 Cargo 声明的 default 入口。最终整图核对会拒绝任何额外 feature。
            if features.contains("default") && !dependency.features.contains("default") {
                missing.insert(format!("{alias}/default"));
            } else {
                missing.extend(
                    features
                        .iter()
                        .filter(|&&feature| !dependency.features.contains(feature))
                        .map(|feature| format!("{alias}/{feature}")),
                );
            }
        }
    }
    missing.into_iter().collect()
}

fn test_targets(baseline: &Graph, selected: &BTreeSet<&str>) -> Option<Vec<String>> {
    let mut targets = BTreeSet::new();
    for &root in &baseline.roots {
        let build = &baseline.units[root].build;
        if !selected.contains(build.package.as_str()) {
            continue;
        }
        // 显式点名 example 会改变默认的只编译语义。
        if build.mode != "test" {
            return None;
        }
        let [kind] = build.target.kind.as_slice() else {
            return None;
        };
        let selector = match kind.as_str() {
            "lib" => ("--lib", None),
            "bin" if super::valid_package(&build.target.name) => {
                ("--bin", Some(&build.target.name))
            }
            "test" if super::valid_package(&build.target.name) => {
                ("--test", Some(&build.target.name))
            }
            _ => return None,
        };
        targets.insert(selector);
    }
    (!targets.is_empty()).then(|| {
        targets
            .into_iter()
            .flat_map(|(flag, name)| std::iter::once(flag.to_owned()).chain(name.cloned()))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn unit(package: &str, target: &str, kind: &str, mode: &str, deps: Value) -> Value {
        json!({
            "pkg_id": package,
            "target": {"name": target, "kind": [kind]},
            "mode": mode,
            "platform": null,
            "profile": {"name": "test"},
            "features": [],
            "dependencies": deps,
        })
    }

    fn graph(roots: &[usize], units: &[Value]) -> Graph {
        let input = json!({"version": 1, "roots": roots, "units": units});
        Graph::parse(&serde_json::to_vec(&input).unwrap()).unwrap()
    }

    #[test]
    fn scoped_build_keeps_dependency_configuration_and_compile_only_targets() {
        let mut sha = unit("dep", "digest", "lib", "build", json!([]));
        sha["features"] = json!(["default"]);
        let old_edge = json!([{"index": 0, "extern_crate_name": "checksum", "public": false}]);
        let suite = unit("member", "suite", "test", "test", old_edge.clone());
        let example = unit("member", "demo", "example", "build", old_edge);
        let unrelated = unit("other", "other", "lib", "test", json!([]));
        let doctest = unit("member", "lib", "lib", "doctest", json!([]));
        let baseline = graph(
            &[1, 2, 3, 4],
            &[
                sha.clone(),
                suite.clone(),
                example.clone(),
                unrelated,
                doctest,
            ],
        );
        let selected = BTreeSet::from(["member"]);
        let mut ids = UnitIds::new();
        let expected = baseline.scope(&mut ids, &selected).unwrap();

        let mut suite = suite;
        let mut example = example;
        suite["dependencies"][0]["index"] = json!(2);
        example["dependencies"][0]["index"] = json!(2);
        let candidate = [example, suite, sha];
        let equivalent = |ids: &mut UnitIds, roots: &[usize], units: &[Value]| {
            graph(roots, units)
                .scope(ids, &selected)
                .unwrap()
                .preserves(&expected)
        };
        assert!(equivalent(&mut ids, &[0, 1], &candidate));
        assert!(!equivalent(&mut ids, &[1], &candidate));
        assert!(test_targets(&baseline, &selected).is_none());

        let mut changed = candidate.clone();
        changed[2]["features"] = json!([]);
        assert!(!equivalent(&mut ids, &[0, 1], &changed));
        let mut changed = candidate.clone();
        changed[2]["profile"]["opt_level"] = json!("2");
        assert!(!equivalent(&mut ids, &[0, 1], &changed));
        let mut changed = candidate.clone();
        changed[1]["dependencies"][0]["public"] = json!(true);
        assert!(!equivalent(&mut ids, &[0, 1], &changed));
        let mut with_extra = candidate.to_vec();
        with_extra.push(unit("other", "gated", "bin", "test", json!([])));
        assert!(!equivalent(&mut ids, &[0, 1, 3], &with_extra));
    }

    #[test]
    fn dependency_features_can_be_restored_through_a_declared_alias() {
        let mut original_dep = unit("digest_id", "digest", "lib", "build", json!([]));
        original_dep["features"] = json!(["alloc", "default"]);
        let consumer = unit(
            "member_id",
            "test_suite",
            "test",
            "test",
            json!([{"index": 0, "extern_crate_name": "proof_digest"}]),
        );
        let baseline = graph(&[1], &[original_dep, consumer.clone()]);
        let candidate = graph(
            &[1],
            &[
                unit("digest_id", "digest", "lib", "build", json!([])),
                consumer,
            ],
        );
        let member = Package {
            id: "member_id".to_owned(),
            name: "member".to_owned(),
            dependencies: vec![Dependency {
                name: "digest".to_owned(),
                rename: Some("proof-digest".to_owned()),
            }],
        };
        let selected = BTreeMap::from([("member_id", &member)]);
        assert_eq!(
            missing_features(&baseline, &candidate, &selected),
            ["proof-digest/default"]
        );
        assert_eq!(
            test_targets(&baseline, &BTreeSet::from(["member_id"])).unwrap(),
            ["--test", "test_suite"]
        );
    }
}
