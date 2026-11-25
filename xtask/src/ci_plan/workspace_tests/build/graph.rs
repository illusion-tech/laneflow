//! Cargo `test --no-run` 的构建单元图。

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
pub(super) struct Graph {
    version: u32,
    pub(super) roots: Vec<usize>,
    pub(super) units: Vec<Unit>,
}

#[derive(Deserialize)]
pub(super) struct Unit {
    #[serde(flatten)]
    pub(super) build: Build,
    pub(super) dependencies: Vec<Edge>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Build {
    #[serde(rename = "pkg_id")]
    pub(super) package: String,
    pub(super) target: Target,
    pub(super) mode: String,
    pub(super) features: BTreeSet<String>,
    #[serde(flatten)]
    attributes: BTreeMap<String, Value>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Target {
    pub(super) name: String,
    pub(super) kind: Vec<String>,
    #[serde(flatten)]
    attributes: BTreeMap<String, Value>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Edge {
    index: usize,
    pub(super) extern_crate_name: String,
    #[serde(flatten)]
    attributes: BTreeMap<String, Value>,
}

type UnitKey = (Vec<u8>, Vec<Vec<u8>>);
pub(super) type UnitIds = BTreeMap<UnitKey, usize>;

pub(super) struct GraphScope {
    units: BTreeSet<usize>,
    roots: BTreeSet<usize>,
    selected: BTreeSet<usize>,
}

impl GraphScope {
    pub(super) fn preserves(&self, baseline: &Self) -> bool {
        !baseline.selected.is_empty()
            && self.selected == baseline.selected
            && self.roots.is_subset(&baseline.roots)
            && self.units.is_subset(&baseline.units)
            && self.units.len() < baseline.units.len()
    }

    pub(super) fn units(&self) -> usize {
        self.units.len()
    }

    pub(super) fn selected_roots(&self) -> usize {
        self.selected.len()
    }
}

#[derive(Clone, Copy)]
enum Visit {
    Unseen,
    Visiting,
    Interned(usize),
}

impl Graph {
    pub(super) fn read(args: &[String]) -> Result<Self, String> {
        // 只在离线解析子进程启用 Cargo 的图导出。nextest 的编译沿用稳定版参数。
        let output = Command::new("cargo")
            .env("RUSTC_BOOTSTRAP", "1")
            .args([
                "test",
                "--no-run",
                "--locked",
                "--offline",
                "-Z",
                "unstable-options",
                "--unit-graph",
            ])
            .args(args)
            .output()
            .map_err(|error| format!("无法导出 Cargo 构建图：{error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Cargo 构建图导出失败（{}）：{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Self::parse(&output.stdout)
    }

    pub(super) fn parse(input: &[u8]) -> Result<Self, String> {
        let mut graph: Self = serde_json::from_slice(input)
            .map_err(|error| format!("无法解析 Cargo 构建图：{error}"))?;
        if graph.version != 1 {
            return Err(format!("不支持的 Cargo 构建图版本 {}", graph.version));
        }
        let dependencies = graph.units.iter().flat_map(|unit| &unit.dependencies);
        if graph
            .roots
            .iter()
            .copied()
            .chain(dependencies.map(|edge| edge.index))
            .any(|index| index >= graph.units.len())
        {
            return Err("Cargo 构建图索引越界".to_owned());
        }
        // Cargo 导出的 doctest 根由独立步骤覆盖，nextest 不从这些根开始构建。
        graph
            .roots
            .retain(|&index| graph.units[index].build.mode != "doctest");
        Ok(graph)
    }

    pub(super) fn dependency(&self, edge: &Edge) -> &Build {
        &self.units[edge.index].build
    }

    pub(super) fn scope(
        &self,
        ids: &mut UnitIds,
        selected: &BTreeSet<&str>,
    ) -> Result<GraphScope, String> {
        let mut visits = vec![Visit::Unseen; self.units.len()];
        let mut roots = BTreeSet::new();
        let mut selected_roots = BTreeSet::new();
        for &index in &self.roots {
            let id = self.intern(index, &mut visits, ids)?;
            roots.insert(id);
            if selected.contains(self.units[index].build.package.as_str()) {
                selected_roots.insert(id);
            }
        }
        Ok(GraphScope {
            units: visits
                .into_iter()
                .filter_map(|visit| match visit {
                    Visit::Interned(id) => Some(id),
                    _ => None,
                })
                .collect(),
            roots,
            selected: selected_roots,
        })
    }

    fn intern(
        &self,
        index: usize,
        visits: &mut [Visit],
        ids: &mut UnitIds,
    ) -> Result<usize, String> {
        match visits.get(index).ok_or("Cargo 构建图的依赖索引越界")? {
            Visit::Interned(id) => return Ok(*id),
            Visit::Visiting => return Err("Cargo 构建图包含循环依赖".to_owned()),
            Visit::Unseen => visits[index] = Visit::Visiting,
        }
        let unit = &self.units[index];
        let mut edges = Vec::with_capacity(unit.dependencies.len());
        for edge in &unit.dependencies {
            let child = self.intern(edge.index, visits, ids)?;
            // 保留边的所有属性，只替换依赖的位置索引。
            edges.push(
                serde_json::to_vec(&(child, &edge.extern_crate_name, &edge.attributes))
                    .map_err(|error| error.to_string())?,
            );
        }
        edges.sort_unstable();
        let build = serde_json::to_vec(&unit.build).map_err(|error| error.to_string())?;
        let next = ids.len();
        let id = *ids.entry((build, edges)).or_insert(next);
        visits[index] = Visit::Interned(id);
        Ok(id)
    }
}
