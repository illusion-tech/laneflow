//! CI 受影响验证计划器（#1464 G2 第一批，设计合同 `docs/design/ci-affected-verification-plan.md`）。
//!
//! 纯计算：读变更文件清单、`cargo metadata` 的 workspace 依赖图与额外输入
//! 注册表（`xtask/data/ci-extra-inputs.toml`），产出受影响包集与各验证任务的
//! 执行范围。PR 的 Rust checks、Evidence、Placement、Urban 消费对应任务并核对步骤结果。
//!
//! 失败关闭语义（合同 §4）：读取或解析失败由命令以非零退出承载，调用方按
//! 全量处理；文件归类落入「未知」态时直接扩大为完整适用套件；「成功计算后
//! 某任务确实不适用」与「计算失败」严格区分。
//!
//! 注册表是归类的事实依据，其可靠性在每次生成计划前自检（合同 §3.2）：对
//! 全仓 `include_str!` / `include_bytes!` 清点并与注册表逐位对账。对账完成
//! 但不一致时，受影响计划不成立——退回全量并列出差异；对账本身失败属计算
//! 失败，按上述非零退出承载。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

mod fuzz_comparison;
mod specialists;
mod workspace_tests;

pub(crate) use workspace_tests::run as run_workspace_tests;

const REGISTRY_PATH: &str = "xtask/data/ci-extra-inputs.toml";
const REGISTRY_TOML: &str = include_str!("../data/ci-extra-inputs.toml");

// ---------------------------------------------------------------------------
// 额外输入注册表
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    schema_version: u32,
    /// 已人工核对的非字面量 include 实参（源文件的仓库相对路径，逐出现
    /// 登记且不去重）：对账时与全仓清点逐出现比对。
    #[serde(default)]
    reviewed_unparsed_includes: Vec<String>,
    #[serde(default)]
    triggers: TriggerSection,
    #[serde(default)]
    embed: Vec<EmbedEntry>,
    /// 运行时读取声明（`std::fs::read` 等非编译期嵌入的仓库文件依赖）：
    /// 无法自动对账，新增运行时读取仓库文件的测试时人工登记。
    #[serde(default)]
    input: Vec<EmbedEntry>,
    #[serde(default)]
    fuzz_comparison_test: Vec<fuzz_comparison::TestTarget>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TriggerSection {
    #[serde(default)]
    full: Vec<String>,
    #[serde(default)]
    not_applicable: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedEntry {
    path: String,
    consumers: Vec<String>,
}

/// 跨包嵌入图：被嵌入文件（仓库相对、正斜杠）→ 嵌入方包名集合。
type EmbedMap = BTreeMap<String, BTreeSet<String>>;

/// 已解析的额外输入注册表。
struct Registry {
    full: Vec<String>,
    not_applicable: Vec<String>,
    /// 跨包嵌入图原始表：计划生成前与全仓清点对账（归类走合并后的
    /// `inputs`）。
    embed: EmbedMap,
    /// 已核对的非字面量 include 实参清单（排序后逐出现保留）。
    reviewed_unparsed: Vec<String>,
    /// 归类用合并输入图：嵌入边 ∪ 运行时读取声明边。
    inputs: EmbedMap,
    fuzz_comparison_tests: Vec<fuzz_comparison::TestTarget>,
}

impl Registry {
    fn parse(text: &str) -> Result<Self, String> {
        let file: RegistryFile =
            toml::from_str(text).map_err(|error| format!("无法解析 {REGISTRY_PATH}：{error}"))?;
        if file.schema_version != 1 {
            return Err(format!("{REGISTRY_PATH} schema_version 须为 1"));
        }
        let mut reviewed_unparsed = file.reviewed_unparsed_includes;
        if reviewed_unparsed.iter().any(String::is_empty) {
            return Err(format!(
                "{REGISTRY_PATH} reviewed_unparsed_includes 含空路径"
            ));
        }
        reviewed_unparsed.sort();
        let mut embed = BTreeMap::new();
        for entry in &file.embed {
            if entry.path.is_empty() || entry.consumers.is_empty() {
                return Err(format!(
                    "{REGISTRY_PATH} embed 条目 `{}` 缺少路径或消费者",
                    entry.path
                ));
            }
            let consumers: BTreeSet<String> = entry.consumers.iter().cloned().collect();
            if consumers.len() != entry.consumers.len() {
                return Err(format!(
                    "{REGISTRY_PATH} embed 条目 `{}` 消费者重复",
                    entry.path
                ));
            }
            if embed.insert(entry.path.clone(), consumers).is_some() {
                return Err(format!("{REGISTRY_PATH} embed 条目 `{}` 重复", entry.path));
            }
        }
        let mut inputs = embed.clone();
        for entry in &file.input {
            if entry.path.is_empty() || entry.consumers.is_empty() {
                return Err(format!(
                    "{REGISTRY_PATH} input 条目 `{}` 缺少路径或消费者",
                    entry.path
                ));
            }
            for consumer in &entry.consumers {
                inputs
                    .entry(entry.path.clone())
                    .or_default()
                    .insert(consumer.clone());
            }
        }
        Ok(Self {
            full: file.triggers.full,
            not_applicable: file.triggers.not_applicable,
            embed,
            reviewed_unparsed,
            inputs,
            fuzz_comparison_tests: file.fuzz_comparison_test,
        })
    }
}

// ---------------------------------------------------------------------------
// 极简 glob：仅支持 `**`（跨目录段）、`*`（段内）与字面量
// ---------------------------------------------------------------------------

fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    match_segments(&pattern, &path)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            if rest.is_empty() {
                return true;
            }
            (0..=path.len()).any(|skip| match_segments(rest, &path[skip..]))
        }
        Some((first, rest)) => match path.split_first() {
            None => false,
            Some((head, tail)) => segment_match(first, head) && match_segments(rest, tail),
        },
    }
}

fn segment_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    chars_match(&pattern, &text)
}

fn chars_match(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((&'*', rest)) => (0..=text.len()).any(|skip| chars_match(rest, &text[skip..])),
        Some((&first, rest)) => match text.split_first() {
            Some((&head, tail)) => first == head && chars_match(rest, tail),
            None => false,
        },
    }
}

// ---------------------------------------------------------------------------
// workspace 依赖图
// ---------------------------------------------------------------------------

/// `cargo metadata` 的最小子集（仅 workspace 成员；容忍未来新增字段）。
#[derive(Deserialize)]
struct MetadataDoc<P = PackageDoc> {
    packages: Vec<P>,
    #[allow(dead_code)]
    workspace_root: PathBuf,
}

#[derive(Deserialize)]
struct PackageDoc {
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    dependencies: Vec<DepDoc>,
}

#[derive(Deserialize)]
struct DepDoc {
    name: String,
}

/// workspace 成员视图：包名 → 仓库相对根目录（正斜杠）；反向邻接
/// （被依赖者 → 依赖者）。第一阶段全部边同权（普通 / 构建 / 开发依赖），
/// 保守多选（合同 §3.1；dev 边类型化属后续批次）。
struct Workspace {
    roots: BTreeMap<String, String>,
    dependents: BTreeMap<String, BTreeSet<String>>,
}

impl Workspace {
    fn parse(metadata_json: &str) -> Result<Self, String> {
        let doc: MetadataDoc = serde_json::from_str(metadata_json)
            .map_err(|error| format!("无法解析 cargo metadata：{error}"))?;
        let mut roots = BTreeMap::new();
        for package in &doc.packages {
            let dir = package
                .manifest_path
                .parent()
                .ok_or_else(|| format!("包 `{}` 的 manifest 无父目录", package.name))?;
            let relative = repo_relative(&doc.workspace_root, dir)?;
            if roots.insert(package.name.clone(), relative).is_some() {
                return Err(format!("workspace 内包名重复：{}", package.name));
            }
        }
        let names: BTreeSet<&str> = roots.keys().map(String::as_str).collect();
        let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for package in &doc.packages {
            for dependency in &package.dependencies {
                if names.contains(dependency.name.as_str()) {
                    dependents
                        .entry(dependency.name.clone())
                        .or_default()
                        .insert(package.name.clone());
                }
            }
        }
        Ok(Self { roots, dependents })
    }

    /// 文件所属包：最长根目录前缀匹配（成员根目录无嵌套，前缀 + `/` 判定）。
    fn package_of(&self, path: &str) -> Option<&str> {
        self.roots
            .iter()
            .filter(|(_, root)| {
                root.len() < path.len()
                    && path.starts_with(root.as_str())
                    && path.as_bytes()[root.len()] == b'/'
            })
            .max_by_key(|(_, root)| root.len())
            .map(|(name, _)| name.as_str())
    }

    /// 成员清单的算法规则（合同 §3.1 前提守护）：任何成员的 `Cargo.toml`
    /// 变更会改写 feature 合并结果，直接全量。
    fn is_member_manifest(&self, path: &str) -> bool {
        self.roots.values().any(|root| {
            path.len() == root.len() + "/Cargo.toml".len() && {
                let (head, tail) = path.split_at(root.len());
                head == root && tail == "/Cargo.toml"
            }
        })
    }

    /// 传递反向闭包：种子自身 ∪ 全部传递消费者。
    fn closure(&self, seeds: &BTreeSet<String>) -> BTreeSet<String> {
        let mut affected = seeds.clone();
        let mut queue: Vec<String> = seeds.iter().cloned().collect();
        while let Some(current) = queue.pop() {
            if let Some(dependents) = self.dependents.get(&current) {
                for dependent in dependents {
                    if affected.insert(dependent.clone()) {
                        queue.push(dependent.clone());
                    }
                }
            }
        }
        affected
    }
}

/// 绝对路径 → 仓库相对路径（正斜杠）；不在仓库内时报错（失败关闭）。
fn repo_relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "路径 `{}` 不在 workspace `{}` 内",
            path.display(),
            root.display()
        )
    })?;
    let out = normal_components(relative).join("/");
    if out.is_empty() {
        return Err(format!("路径 `{}` 等于 workspace 根", path.display()));
    }
    Ok(out)
}

/// 只取 Normal 组件（剔除 Windows 的盘符前缀与根目录组件），正斜杠连接语义。
fn normal_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 变更归类（合同 §4 三态）
// ---------------------------------------------------------------------------

/// 单个变更文件的归类结果。
#[derive(Clone, Debug, PartialEq, Eq)]
enum FileState {
    /// 属于某包：种子。
    Package(String),
    /// 注册表输入边（编译期嵌入或运行时读取）：种子 = 所有者包（若属于某
    /// 包）∪ 注册表声明的消费者。
    Input {
        owner: Option<String>,
        consumers: Vec<String>,
    },
    /// 命中全量触发器（模式名或成员清单规则）。
    FullTrigger(String),
    /// 有明确依据不适用（命中的模式）。
    NotApplicable(String),
    /// 不能确定：扩大为全量。
    Unknown,
}

fn classify(path: &str, workspace: &Workspace, registry: &Registry) -> FileState {
    // 顺序即优先级：全量触发与成员清单规则先于输入边（成员 manifest 的跨包
    // 读取不得吞掉 feature 合并前提守护）；显式不适用规则先于通用包目录归属
    // ——xtask/data 等包内数据由常驻校验承载（#1459），包归属不得复活重活。
    if let Some(pattern) = registry
        .full
        .iter()
        .find(|pattern| glob_match(pattern, path))
    {
        return FileState::FullTrigger(pattern.clone());
    }
    if workspace.is_member_manifest(path) {
        return FileState::FullTrigger("成员 Cargo.toml（feature 合并前提守护）".to_owned());
    }
    if let Some(consumers) = registry.inputs.get(path) {
        let owner = workspace.package_of(path).map(str::to_owned);
        return FileState::Input {
            owner,
            consumers: consumers.iter().cloned().collect(),
        };
    }
    if let Some(pattern) = registry
        .not_applicable
        .iter()
        .find(|pattern| glob_match(pattern, path))
    {
        return FileState::NotApplicable(pattern.clone());
    }
    if let Some(name) = workspace.package_of(path) {
        return FileState::Package(name.to_owned());
    }
    FileState::Unknown
}

// ---------------------------------------------------------------------------
// 验证任务表（任务身份：包 × feature 配置 × 检查类型；合同 §2）
// ---------------------------------------------------------------------------

/// 任务包集合的取法。
enum TaskPackages {
    /// 随受影响集（全量模式 = 全部成员）。
    Affected,
    /// 随受影响集排除固定成员（结构性分流：被排除包的验证由其他任务承载）。
    AffectedExclude(&'static [&'static str]),
    /// 固定配置任务：只要存在任何 Rust 种子即执行（与现行 rust 布尔门同语义；
    /// 配置身份固定，不随闭包收窄——收窄评估留给影子期数据）。
    Fixed(&'static [&'static str]),
    /// 全量或受影响集与门集合相交时执行，执行范围为固定集合。
    Gate {
        gate: &'static [&'static str],
        packages: &'static [&'static str],
    },
}

impl TaskPackages {
    /// 任一目标包受影响即运行整组，保持专项原有的 feature 与目标覆盖。
    const fn when_affected(packages: &'static [&'static str]) -> Self {
        Self::Gate {
            gate: packages,
            packages,
        }
    }
}

struct TaskDef {
    name: &'static str,
    packages: TaskPackages,
    note: &'static str,
}

/// 与 ci.yml 重活 job 的现行步骤一一对应；接线批次只改消费方式不改表。
const TASKS: &[TaskDef] = &[
    TaskDef {
        name: "clippy-default",
        packages: TaskPackages::Affected,
        note: "cargo clippy --all-targets -D warnings（受影响包）",
    },
    TaskDef {
        name: "clippy-execution-evidence",
        packages: TaskPackages::when_affected(&["laneflow-runtime", "laneflow-urban-harness"]),
        note: "execution-evidence feature 的 clippy 护栏，配置身份固定",
    },
    TaskDef {
        name: "test-default",
        packages: TaskPackages::AffectedExclude(workspace_tests::EXCLUDED_PACKAGES),
        note: "workspace nextest 补集步；corridor_traffic_smoke 目标由专项承载",
    },
    TaskDef {
        name: "test-execution-evidence",
        packages: TaskPackages::when_affected(&["laneflow-runtime"]),
        note: "--lib 精确目标（#1462）",
    },
    TaskDef {
        name: "clippy-motion-trace",
        packages: TaskPackages::Fixed(&["laneflow-runtime", "laneflow-scenario"]),
        note: "motion-trace 只读运动诊断 feature 的 clippy 护栏（#1500），配置身份固定",
    },
    TaskDef {
        name: "test-motion-trace",
        packages: TaskPackages::Fixed(&["laneflow-runtime", "laneflow-scenario"]),
        note: "Runtime --lib 对照回归与 signalized_corridor_yellow_trace 精确目标（#1500）",
    },
    TaskDef {
        name: "test-doctest",
        packages: TaskPackages::Affected,
        note: "doctest 独立入口，nextest 不执行 doctest",
    },
    TaskDef {
        name: "compile-bevy-examples",
        packages: TaskPackages::Gate {
            gate: &[
                "laneflow-bevy",
                "laneflow-runtime",
                "laneflow-spatial",
                "laneflow-scenario",
                "laneflow-format",
                "laneflow-format-mmap",
                "laneflow-static-contract",
                "laneflow-static-network",
                "laneflow-compiler",
            ],
            packages: &["laneflow-bevy"],
        },
        note: "native-example 编译门（现行 bevy 路径过滤器的 crate 口径）",
    },
    TaskDef {
        name: "test-urban-evidence",
        packages: TaskPackages::when_affected(&[
            "laneflow-urban-harness",
            "laneflow-urban-generator",
        ]),
        note: "Urban harness 或 generator 受影响时重测两包的默认配置（#1108）",
    },
    TaskDef {
        name: "test-corridor-smoke",
        packages: TaskPackages::when_affected(&["laneflow-bevy"]),
        note: "Bevy 受影响时运行 --test corridor_traffic_smoke（#1462）",
    },
    TaskDef {
        name: "placement-fixtures",
        packages: TaskPackages::when_affected(&[
            "laneflow-runtime",
            "laneflow-scenario",
            "laneflow-junction-generator",
        ]),
        note: "Runtime、Scenario 或 junction-generator 受影响时运行三包的 placement-fixtures clippy 与测试",
    },
    TaskDef {
        name: "test-urban-adapter",
        packages: TaskPackages::when_affected(&["laneflow-urban-harness"]),
        note: "Urban harness 受影响时运行其 adapter 配置测试",
    },
    TaskDef {
        name: "wire-audit",
        packages: TaskPackages::Affected,
        note: "wire 审计与运行时架构检查扫描整个 workspace",
    },
];

// ---------------------------------------------------------------------------
// 计划生成
// ---------------------------------------------------------------------------

/// 一次验证计划。
struct Plan {
    full: bool,
    reasons: Vec<String>,
    seeds: BTreeSet<String>,
    affected: BTreeSet<String>,
    tasks: Vec<TaskPlan>,
    classification: Vec<(String, FileState)>,
    /// 生成该计划时的注册表对账结论。
    registry_check: RegistryCheck,
}

struct TaskPlan {
    name: &'static str,
    packages: Vec<String>,
    note: &'static str,
}

fn build_plan(
    files: &BTreeSet<String>,
    workspace: &Workspace,
    registry: &Registry,
    registry_check: RegistryCheck,
) -> Plan {
    let mut reasons = Vec::new();
    // 对账不一致：受影响计划的前提不成立，差异逐条入原因并因此全量。
    let drift: &[String] = match &registry_check {
        RegistryCheck::Drift(items) => items,
        RegistryCheck::Consistent { .. } => &[],
    };
    for item in drift {
        reasons.push(format!("注册表对账不一致，计划退回全量：{item}"));
    }
    let mut seeds = BTreeSet::new();
    let mut classification = Vec::with_capacity(files.len());

    for path in files {
        let state = classify(path, workspace, registry);
        match &state {
            FileState::Package(name) => {
                seeds.insert(name.clone());
            }
            FileState::Input { owner, consumers } => {
                if let Some(owner) = owner {
                    seeds.insert(owner.clone());
                }
                seeds.extend(consumers.iter().cloned());
            }
            FileState::FullTrigger(reason) => {
                reasons.push(format!("全量触发：{path}（{reason}）"));
            }
            FileState::NotApplicable(_) => {}
            FileState::Unknown => reasons.push(format!("未知路径，扩大为全量：{path}")),
        }
        classification.push((path.clone(), state));
    }

    // 无种子且无触发（纯不适用路径）：受影响集为空，全部任务跳过——与现行
    // 纯文档路径显式 skip 同语义。
    let full = !reasons.is_empty();
    let has_rust_seed = !seeds.is_empty();
    let affected = if full {
        workspace.roots.keys().cloned().collect()
    } else if has_rust_seed {
        workspace.closure(&seeds)
    } else {
        BTreeSet::new()
    };
    let task_pool: Vec<String> = if full {
        workspace.roots.keys().cloned().collect()
    } else {
        affected.iter().cloned().collect()
    };

    let tasks = TASKS
        .iter()
        .map(|def| {
            let packages: Vec<String> = if !has_rust_seed && !full {
                Vec::new()
            } else {
                match &def.packages {
                    TaskPackages::Affected => task_pool.clone(),
                    TaskPackages::AffectedExclude(excluded) => task_pool
                        .iter()
                        .filter(|name| !excluded.contains(&name.as_str()))
                        .cloned()
                        .collect(),
                    TaskPackages::Fixed(fixed) => {
                        fixed.iter().map(|name| (*name).to_owned()).collect()
                    }
                    TaskPackages::Gate { gate, packages } => {
                        let gate_open = full || gate.iter().any(|name| affected.contains(*name));
                        if gate_open {
                            packages.iter().map(|name| (*name).to_owned()).collect()
                        } else {
                            Vec::new()
                        }
                    }
                }
            };
            TaskPlan {
                name: def.name,
                packages,
                note: def.note,
            }
        })
        .collect();

    Plan {
        full,
        reasons,
        seeds,
        affected,
        tasks,
        classification,
        registry_check,
    }
}

// ---------------------------------------------------------------------------
// 跨包嵌入扫描（注册表一致性的事实来源）
// ---------------------------------------------------------------------------

/// 对 workspace 成员源码全量清点 `include_str!` / `include_bytes!`，返回
/// （仓库相对路径 → 嵌入方包名 集合，含同包与跨包由调用方过滤；
/// 非字面量实参的源文件清单）。非字面量实参（如
/// `concat!(env!("CARGO_MANIFEST_DIR"), …)`）无法静态解析，不入图、逐出现
/// 记入清单；清单与注册表已核对条目的对账见 [`check_registry`]。扫描失败
/// 即失败关闭。
fn scan_embeds(workspace: &Workspace, repo_root: &Path) -> Result<(EmbedMap, Vec<String>), String> {
    let mut map: EmbedMap = BTreeMap::new();
    let mut unparsed: Vec<String> = Vec::new();
    for dir in ["crates", "tools", "xtask"] {
        walk_sources(&repo_root.join(dir), &mut |file| {
            let text = std::fs::read_to_string(file)
                .map_err(|error| format!("无法读取 {}: {error}", file.display()))?;
            let parsed = syn::parse_file(&text)
                .map_err(|error| format!("无法解析 {}: {error}", file.display()))?;
            let relative = repo_relative(repo_root, file)?;
            let embedder = workspace
                .package_of(&relative)
                .ok_or_else(|| format!("源文件 {relative} 不属于任何 workspace 成员"))?
                .to_owned();
            for target in collect_include_literals(&parsed) {
                if target == UNPARSED_INCLUDE {
                    unparsed.push(relative.clone());
                    continue;
                }
                let resolved = resolve_embed_target(repo_root, file, &target)?;
                map.entry(resolved).or_default().insert(embedder.clone());
            }
            Ok(())
        })?;
    }
    // 逐出现记录（不去重）：同一已允许文件新增另一处非字面量实参时，
    // 清单长度变化使一致性断言抓红。
    unparsed.sort();
    Ok((map, unparsed))
}

/// 注册表对账结论（合同 §3.2）：嵌入图与已核对清单是归类的依据，计划
/// 只能在它们与全仓清点一致时成立。
#[derive(Debug, PartialEq, Eq)]
enum RegistryCheck {
    /// 逐位一致；`cross_package_edges` 为跨包嵌入边数。
    Consistent { cross_package_edges: usize },
    /// 不一致：每条差异一项。带差异的受影响计划不成立，退回全量。
    Drift(Vec<String>),
}

/// 计划生成前的注册表自检。差异各自成项：清点到而未登记的跨包嵌入
/// （对应文件变更时会漏选嵌入方）、登记了却已不存在或嵌入方不符的条目
/// （登记失效）、嵌入了却仍判不适用的 README、未核对的非字面量实参出现位置。对账
/// 本身做不完（读不了、解析不了）按计算失败返回错误，不产出计划。
fn check_registry(
    workspace: &Workspace,
    repo_root: &Path,
    registry: &Registry,
) -> Result<RegistryCheck, String> {
    let (scanned, unparsed) = scan_embeds(workspace, repo_root)?;
    let cross = cross_package_embeds(&scanned, workspace);
    let mut drift = Vec::new();
    for (path, embedders) in &scanned {
        if path.rsplit('/').next() == Some("README.md")
            && let FileState::NotApplicable(pattern) = classify(path, workspace, registry)
        {
            drift.push(format!(
                "README 已被嵌入却命中不适用规则 {pattern}：{path}（嵌入方 {}）",
                join(embedders)
            ));
        }
    }
    for (target, embedders) in &cross {
        match registry.embed.get(target) {
            None => drift.push(format!(
                "未登记的跨包嵌入：{target}（嵌入方 {}）",
                join(embedders)
            )),
            Some(registered) if registered != embedders => drift.push(format!(
                "跨包嵌入的嵌入方与登记不符：{target}（登记 {}；实测 {}）",
                join(registered),
                join(embedders)
            )),
            Some(_) => {}
        }
    }
    for (target, consumers) in &registry.embed {
        if !cross.contains_key(target) {
            drift.push(format!(
                "登记的跨包嵌入已不存在：{target}（登记 {}）",
                join(consumers)
            ));
        }
    }
    if unparsed != registry.reviewed_unparsed {
        drift.push(format!(
            "非字面量 include 实参与已核对清单不一致（逐出现）：登记 [{}]；实测 [{}]",
            registry.reviewed_unparsed.join(", "),
            unparsed.join(", ")
        ));
    }
    if drift.is_empty() {
        Ok(RegistryCheck::Consistent {
            cross_package_edges: cross.len(),
        })
    } else {
        Ok(RegistryCheck::Drift(drift))
    }
}

/// 只保留跨包嵌入：目标不属于任何成员，或所有者 ≠ 某个嵌入方；值保留该
/// 目标的全部嵌入方（含所有者自身的同包嵌入）。
fn cross_package_embeds(scanned: &EmbedMap, workspace: &Workspace) -> EmbedMap {
    scanned
        .iter()
        .filter(|(target, embedders)| {
            embedders.iter().any(|embedder| {
                workspace
                    .package_of(target)
                    .map(|owner| owner != embedder)
                    .unwrap_or(true)
            })
        })
        .map(|(target, embedders)| (target.clone(), embedders.clone()))
        .collect()
}

fn walk_sources(
    dir: &Path,
    visit: &mut dyn FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("无法读取目录 {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法读取目录 {}: {error}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk_sources(&path, visit)?;
        } else if name.ends_with(".rs") {
            visit(&path)?;
        }
    }
    Ok(())
}

/// 收集一个文件里全部 `include_str!` / `include_bytes!` 的字符串实参：直接
/// 形态、`std::` 前缀形态、嵌在其他宏实参里的形态（如
/// `assert_eq!(include_str!(…), …)`）与相邻字面量拼接。syn 的访问器只到达
/// 宏调用本身，嵌套形态须在 token 层递归。
fn collect_include_literals(file: &syn::File) -> Vec<String> {
    collect_file_inputs(file, false)
}

fn collect_fuzz_includes(file: &syn::File) -> Vec<String> {
    collect_file_inputs(file, true)
}

fn collect_file_inputs(file: &syn::File, include_source: bool) -> Vec<String> {
    struct Visitor {
        targets: Vec<String>,
        include_source: bool,
    }
    impl syn::visit::Visit<'_> for Visitor {
        fn visit_meta_list(&mut self, meta: &syn::MetaList) {
            // cfg_attr 等属性列表保留为 token，syn 不会将其中的 doc 嵌入解析为表达式。
            scan_nested_includes(&meta.tokens, &mut self.targets, self.include_source);
        }

        fn visit_meta_name_value(&mut self, meta: &syn::MetaNameValue) {
            if self.include_source && meta.path.is_ident("path") {
                self.targets.push(UNPARSED_INCLUDE.to_owned());
            }
            syn::visit::visit_meta_name_value(self, meta);
        }

        fn visit_macro(&mut self, mac: &syn::Macro) {
            let ident = mac.path.segments.last().map(|s| s.ident.to_string());
            if self.include_source && ident.as_deref() == Some("include") {
                self.targets.push(UNPARSED_INCLUDE.to_owned());
            } else if matches!(ident.as_deref(), Some("include_str" | "include_bytes")) {
                self.targets.push(direct_include_argument(&mac.tokens));
            }
            scan_nested_includes(&mac.tokens, &mut self.targets, self.include_source);
        }
    }
    let mut visitor = Visitor {
        targets: Vec::new(),
        include_source,
    };
    syn::visit::visit_file(&mut visitor, file);
    visitor.targets
}

const UNPARSED_INCLUDE: &str = "\u{0}unparsed-include";

/// include 宏自身实参的提取：只接受纯字面量序列（含相邻拼接）；出现任何
/// 标识符或组（如 `concat!(env!(…), …)`）即非字面量形态，记哨兵。
fn direct_include_argument(tokens: &proc_macro2::TokenStream) -> String {
    let mut literal = String::new();
    for token in tokens.clone() {
        match token {
            proc_macro2::TokenTree::Literal(lit) => push_literal_text(&lit, &mut literal),
            proc_macro2::TokenTree::Group(_) | proc_macro2::TokenTree::Ident(_) => {
                return UNPARSED_INCLUDE.to_owned();
            }
            proc_macro2::TokenTree::Punct(_) => {}
        }
    }
    if literal.is_empty() {
        UNPARSED_INCLUDE.to_owned()
    } else {
        literal
    }
}

/// token 层递归：捕获嵌在其他宏实参里的 `include_str!(…)` / `include_bytes!(…)`
/// 调用。精确识别「标识符 → 可选 `!` → 恰好一个实参组」，实参组复用
/// `direct_include_argument` 解析，处理完在外层紧随该组之后继续——不越过
/// 自身实参边界吞掉外层的运算符、比较字面量或后续 include；并继续深入
/// 其余组。
fn scan_nested_includes(
    tokens: &proc_macro2::TokenStream,
    targets: &mut Vec<String>,
    include_source: bool,
) {
    let tokens: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            proc_macro2::TokenTree::Group(group) => {
                scan_nested_includes(&group.stream(), targets, include_source);
                index += 1;
            }
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if include_source
                    && name == "path"
                    && matches!(tokens.get(index + 1), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=')
                {
                    targets.push(UNPARSED_INCLUDE.to_owned());
                }
                if include_source && name == "include" {
                    targets.push(UNPARSED_INCLUDE.to_owned());
                }
                if !matches!(name.as_str(), "include_str" | "include_bytes") {
                    index += 1;
                    continue;
                }
                // 标识符 → 可选 `!` → 恰好一个实参组；嵌套位置总是完整调用
                // 形态（syn 只对外层宏剥离定界符）。形态不符记哨兵（失败关闭）。
                let mut cursor = index + 1;
                if matches!(tokens.get(cursor), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '!')
                {
                    cursor += 1;
                }
                match tokens.get(cursor) {
                    Some(proc_macro2::TokenTree::Group(group)) => {
                        targets.push(direct_include_argument(&group.stream()));
                        index = cursor + 1;
                    }
                    _ => {
                        targets.push(UNPARSED_INCLUDE.to_owned());
                        index = cursor;
                    }
                }
            }
            _ => index += 1,
        }
    }
}

fn push_literal_text(literal: &proc_macro2::Literal, out: &mut String) {
    // 经 syn 的字面量模型解码（转义序列与 raw string 均正确处理）；
    // 非字符串字面量不贡献路径。
    if let syn::Lit::Str(text) = syn::Lit::new(literal.clone()) {
        out.push_str(&text.value());
    }
}

/// 字面量相对源文件目录解析为仓库相对路径（纯词法归一，不触文件系统）。
fn resolve_embed_target(repo_root: &Path, source: &Path, literal: &str) -> Result<String, String> {
    let base = source.parent().ok_or("源文件无父目录")?;
    let mut stack: Vec<String> = repo_relative(repo_root, base)?
        .split('/')
        .map(str::to_owned)
        .collect();
    for segment in literal.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return Err(format!(
                        "include 路径 `{literal}` 越出仓库根（源 {}）",
                        source.display()
                    ));
                }
            }
            other => stack.push(other.to_owned()),
        }
    }
    let relative = stack.join("/");
    if relative.is_empty() {
        return Err(format!(
            "include 路径 `{literal}` 解析为仓库根（源 {}）",
            source.display()
        ));
    }
    Ok(relative)
}

// ---------------------------------------------------------------------------
// 命令行入口
// ---------------------------------------------------------------------------

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if matches!(args, [flag] if flag == "--pr-specialists") {
        return specialists::produce();
    }
    if let Some((flag, args)) = args.split_first()
        && flag == "--compare-fuzz"
    {
        return fuzz_comparison::run(args);
    }
    let mut rev_range: Option<&str> = None;
    let mut files_path: Option<&str> = None;
    let mut metadata_path: Option<&str> = None;
    let mut dump_embed_map = false;
    let mut shadow = false;
    let mut format = "json";
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--rev-range" => {
                index += 1;
                rev_range = Some(args.get(index).ok_or("ci-plan: --rev-range 缺少值")?);
            }
            "--files" => {
                index += 1;
                files_path = Some(args.get(index).ok_or("ci-plan: --files 缺少值")?);
            }
            "--metadata" => {
                index += 1;
                metadata_path = Some(args.get(index).ok_or("ci-plan: --metadata 缺少值")?);
            }
            "--dump-embed-map" => dump_embed_map = true,
            "--shadow" => shadow = true,
            "--format" => {
                index += 1;
                format = args.get(index).ok_or("ci-plan: --format 缺少值")?;
            }
            other => return Err(format!("ci-plan: 未知参数 {other}")),
        }
        index += 1;
    }

    if shadow {
        if rev_range.is_some() || files_path.is_some() || metadata_path.is_some() || dump_embed_map
        {
            return Err("ci-plan: --shadow 不与其他输入参数并用".to_owned());
        }
        return run_shadow();
    }

    let metadata = match metadata_path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("ci-plan: 无法读取 metadata `{path}`：{error}"))?,
        None => cargo_metadata()?,
    };
    let workspace = Workspace::parse(&metadata)?;
    let registry = Registry::parse(REGISTRY_TOML)?;
    let repo_root = repo_root()?;

    if dump_embed_map {
        let (map, unparsed) = scan_embeds(&workspace, &repo_root)?;
        for (path, embedders) in &map {
            println!(
                "{path}\t{}",
                embedders.iter().cloned().collect::<Vec<_>>().join(",")
            );
        }
        for source in &unparsed {
            println!("# 非字面量 include 实参（人工核对）: {source}");
        }
        return Ok(());
    }

    let files = match (rev_range, files_path) {
        (Some(range), None) => changed_files(range)?,
        (None, Some(path)) => read_files_list(path)?,
        _ => {
            return Err(
                "ci-plan: 需要 --rev-range <base>..<head> 或 --files <清单文件> 之一".to_owned(),
            );
        }
    };

    let registry_check = check_registry(&workspace, &repo_root, &registry)?;
    let plan = build_plan(&files, &workspace, &registry, registry_check);
    match format {
        "json" => print_json(&plan),
        "summary" => print_summary(&plan),
        other => {
            return Err(format!(
                "ci-plan: 未知 --format {other}（可用 json/summary）"
            ));
        }
    }
    Ok(())
}

/// 仓库根 = 当前工作目录的规范化形式（计划器须在仓库根运行）。
fn repo_root() -> Result<PathBuf, String> {
    std::env::current_dir()
        .map_err(|error| format!("ci-plan: 无法确定工作目录：{error}"))?
        .canonicalize()
        .map_err(|error| format!("ci-plan: 无法规范化工作目录：{error}"))
}

fn cargo_metadata() -> Result<String, String> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|error| format!("ci-plan: 无法启动 cargo metadata：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ci-plan: cargo metadata 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("ci-plan: metadata 非 UTF-8：{error}"))
}

/// 完整补丁集（合同 §5 输入语义）：`--name-status -z` 逐 token 解析，
/// 重命名 / 复制的新旧两侧都参与归类。
fn changed_files(rev_range: &str) -> Result<BTreeSet<String>, String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-status", "-z", rev_range])
        .output()
        .map_err(|error| format!("ci-plan: 无法启动 git diff：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ci-plan: git diff {rev_range} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("ci-plan: git diff 路径不是 UTF-8：{error}"))?;
    let tokens: Vec<&str> = text.split('\0').filter(|token| !token.is_empty()).collect();
    let mut files = BTreeSet::new();
    let mut index = 0;
    while index < tokens.len() {
        let status = tokens[index];
        if matches!(status, "A" | "M" | "D" | "T") {
            let path = *tokens
                .get(index + 1)
                .ok_or("ci-plan: git diff -z 输出不完整")?;
            files.insert(path.to_owned());
            index += 2;
        } else if status.starts_with('R') || status.starts_with('C') {
            let new_path = *tokens
                .get(index + 1)
                .ok_or("ci-plan: git diff -z 输出不完整")?;
            let old_path = *tokens
                .get(index + 2)
                .ok_or("ci-plan: git diff -z 输出不完整")?;
            files.insert(new_path.to_owned());
            files.insert(old_path.to_owned());
            index += 3;
        } else {
            return Err(format!(
                "ci-plan: git diff 状态 `{status}` 不可判定（未合并 / 未知），按全量处理"
            ));
        }
    }
    Ok(files)
}

fn read_files_list(path: &str) -> Result<BTreeSet<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("ci-plan: 无法读取清单 `{path}`：{error}"))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn state_json(state: &FileState) -> serde_json::Value {
    match state {
        FileState::Package(name) => serde_json::json!({"state": "package", "package": name}),
        FileState::Input { owner, consumers } => serde_json::json!({
            "state": "input",
            "owner": owner,
            "consumers": consumers,
        }),
        FileState::FullTrigger(reason) => {
            serde_json::json!({"state": "full-trigger", "reason": reason})
        }
        FileState::NotApplicable(pattern) => {
            serde_json::json!({"state": "not-applicable", "pattern": pattern})
        }
        FileState::Unknown => serde_json::json!({"state": "unknown"}),
    }
}

fn plan_json_text(plan: &Plan) -> String {
    let classification: Vec<serde_json::Value> = plan
        .classification
        .iter()
        .map(|(path, state)| {
            let mut value = state_json(state);
            if let serde_json::Value::Object(map) = &mut value {
                map.insert("path".to_owned(), serde_json::json!(path));
            }
            value
        })
        .collect();
    let tasks: Vec<serde_json::Value> = plan
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "name": task.name,
                "packages": task.packages,
                "note": task.note,
            })
        })
        .collect();
    let value = serde_json::json!({
        "mode": if plan.full { "full" } else { "affected" },
        "registry_check": match &plan.registry_check {
            RegistryCheck::Consistent { cross_package_edges } => serde_json::json!({
                "status": "consistent",
                "cross_package_edges": cross_package_edges,
            }),
            RegistryCheck::Drift(_) => serde_json::json!({"status": "drift"}),
        },
        "reasons": plan.reasons,
        "seed_packages": plan.seeds,
        "affected_packages": plan.affected,
        "tasks": tasks,
        "classification": classification,
    });
    serde_json::to_string_pretty(&value).expect("计划序列化不可失败")
}

fn print_json(plan: &Plan) {
    println!("{}", plan_json_text(plan));
}

fn plan_summary_text(plan: &Plan) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if plan.full {
        let _ = writeln!(out, "模式：全量（完整适用套件）");
    } else {
        let _ = writeln!(out, "模式：受影响");
    }
    for reason in &plan.reasons {
        let _ = writeln!(out, "  {reason}");
    }
    if let RegistryCheck::Consistent {
        cross_package_edges,
    } = &plan.registry_check
    {
        let _ = writeln!(out, "注册表对账：一致（跨包嵌入 {cross_package_edges} 条）");
    }
    let _ = writeln!(out, "种子包：{}", join(&plan.seeds));
    let _ = writeln!(out, "受影响包：{}", join(&plan.affected));
    let _ = writeln!(out, "任务：");
    for task in &plan.tasks {
        if task.packages.is_empty() {
            let _ = writeln!(out, "  [跳过] {}（{}）", task.name, task.note);
        } else {
            let _ = writeln!(
                out,
                "  [执行] {} → {}（{}）",
                task.name,
                task.packages.join(","),
                task.note
            );
        }
    }
    let _ = writeln!(out, "归类：");
    for (path, state) in &plan.classification {
        let kind = match state {
            FileState::Package(name) => format!("包 {name}"),
            FileState::Input { owner, consumers } => format!(
                "注册表输入（消费者 {}{}）",
                consumers.join(","),
                owner
                    .clone()
                    .map(|o| format!("；所有者 {o}"))
                    .unwrap_or_default()
            ),
            FileState::FullTrigger(_) => "全量触发".to_owned(),
            FileState::NotApplicable(_) => "不适用".to_owned(),
            FileState::Unknown => "未知→全量".to_owned(),
        };
        let _ = writeln!(out, "  {path} → {kind}");
    }
    out
}

fn print_summary(plan: &Plan) {
    print!("{}", plan_summary_text(plan));
}

// ---------------------------------------------------------------------------
// Actions 事件计划使用合并快照的完整差异。影子模式把计算错误写入 summary，
// PR 专项的全量回退与正式执行选择由 specialists 模块处理。
// ---------------------------------------------------------------------------

fn run_shadow() -> Result<(), String> {
    let markdown = match std::env::var("GITHUB_EVENT_NAME")
        .map_err(|_| "缺少 GITHUB_EVENT_NAME".to_owned())
        .and_then(|event| event_plan(&event))
    {
        Ok(plan) => plan_markdown(
            "影子验证计划（#1464，不改变门禁行为）",
            &plan_summary_text(&plan),
            &plan_json_text(&plan),
        ),
        Err(error) => shadow_failure_markdown(&error),
    };
    append_step_summary(&markdown)
}

fn append_step_summary(markdown: &str) -> Result<(), String> {
    match std::env::var("GITHUB_STEP_SUMMARY") {
        Ok(path) if !path.is_empty() => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| format!("无法写入 GITHUB_STEP_SUMMARY `{path}`：{error}"))?;
            file.write_all(markdown.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .map_err(|error| format!("写入 GITHUB_STEP_SUMMARY `{path}` 失败：{error}"))?;
            Ok(())
        }
        // 本地干跑：无 runner 环境，直接输出到 stdout。
        _ => {
            println!("{markdown}");
            Ok(())
        }
    }
}

fn append_step_outputs(outputs: &str) -> Result<(), String> {
    use std::io::Write as _;
    let path = std::env::var_os("GITHUB_OUTPUT").ok_or("缺少 GITHUB_OUTPUT")?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(outputs.as_bytes()))
        .map_err(|error| format!("无法写入 GITHUB_OUTPUT：{error}"))
}

fn event_plan(event_name: &str) -> Result<Plan, String> {
    let files = event_changed_files(event_name)?;
    let workspace = Workspace::parse(&cargo_metadata()?)?;
    let registry = Registry::parse(REGISTRY_TOML)?;
    let registry_check = check_registry(&workspace, &repo_root()?, &registry)?;
    Ok(build_plan(&files, &workspace, &registry, registry_check))
}

fn event_changed_files(event_name: &str) -> Result<BTreeSet<String>, String> {
    let event_path =
        std::env::var("GITHUB_EVENT_PATH").map_err(|_| "GITHUB_EVENT_PATH 未设置".to_owned())?;
    let payload_text = std::fs::read_to_string(&event_path)
        .map_err(|error| format!("无法读取事件载荷 `{event_path}`：{error}"))?;
    let payload: serde_json::Value = serde_json::from_str(&payload_text)
        .map_err(|error| format!("事件载荷不是合法 JSON：{error}"))?;
    let base = event_base(&payload, event_name)?;
    // 完整差异语义（合同 §5）：合并快照 / 组头对该 base 提交对象的差异。
    let output = std::process::Command::new("git")
        .args(["fetch", "--no-tags", "origin", &base])
        .output()
        .map_err(|error| format!("无法启动 git fetch：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git fetch {base} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    changed_files(&format!("{base}..HEAD"))
}

/// 组装影子 summary 的 markdown（纯函数）。代码围栏必须独立成行：
/// `plan_json_text` 不带尾换行，闭合围栏前显式补换行——否则围栏失效，
/// `</details>` 被吞进代码块，折叠报告不成立。
fn plan_markdown(title: &str, summary_text: &str, json_text: &str) -> String {
    format!(
        "## {title}\n\n```text\n{summary_text}```\n\n<details><summary>计划 JSON</summary>\n\n```json\n{json_text}\n```\n\n</details>\n"
    )
}

/// 从事件载荷提取差异基线提交（纯函数）。
fn event_base(payload: &serde_json::Value, event_name: &str) -> Result<String, String> {
    let sha = match event_name {
        "pull_request" => payload.pointer("/pull_request/base/sha"),
        "merge_group" => payload.pointer("/merge_group/base_sha"),
        other => {
            return Err(format!(
                "事件计划不支持 `{other}`（仅支持 pull_request 与 merge_group）"
            ));
        }
    };
    sha.and_then(serde_json::Value::as_str)
        .filter(|sha| {
            matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("事件载荷缺少完整有效的基线提交 SHA（{event_name}）"))
}

fn shadow_failure_markdown(error: &str) -> String {
    format!(
        "## 影子验证计划（#1464，不改变门禁行为）\n\n⚠️ 影子计划失败：各项检查仍按现行路径门执行。\n\n```text\n{error}\n```\n"
    )
}

fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_METADATA: &str = r##"{
        "packages": [
            {"name": "laneflow-core-lib", "manifest_path": "/repo/crates/core/Cargo.toml", "dependencies": []},
            {"name": "laneflow-mid-lib", "manifest_path": "/repo/crates/mid/Cargo.toml", "dependencies": [{"name": "laneflow-core-lib"}]},
            {"name": "laneflow-leaf-tool", "manifest_path": "/repo/tools/leaf/Cargo.toml", "dependencies": [{"name": "laneflow-core-lib"}]},
            {"name": "laneflow-runtime", "manifest_path": "/repo/crates/runtime/Cargo.toml", "dependencies": [{"name": "laneflow-core-lib"}]},
            {"name": "laneflow-consumer", "manifest_path": "/repo/crates/consumer/Cargo.toml", "dependencies": [{"name": "laneflow-mid-lib"}]},
            {"name": "xtask", "manifest_path": "/repo/xtask/Cargo.toml", "dependencies": []}
        ],
        "workspace_root": "/repo"
    }"##;

    fn fixture_workspace() -> Workspace {
        Workspace::parse(FIXTURE_METADATA).expect("fixture metadata 须可解析")
    }

    fn registry() -> Registry {
        Registry::parse(REGISTRY_TOML).expect("内置注册表须可解析")
    }

    fn files(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|path| (*path).to_owned()).collect()
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// 对账一致的占位结论：只考察归类与任务的测试不关心对账。
    fn consistent() -> RegistryCheck {
        RegistryCheck::Consistent {
            cross_package_edges: 0,
        }
    }

    #[test]
    fn glob_matches_directory_star_star_and_plain_star() {
        assert!(glob_match("docs/**", "docs/a/b.md"));
        assert!(glob_match("docs/**", "docs/a.md"));
        assert!(!glob_match("docs/**", "docsx/a.md"));
        assert!(glob_match("rust-toolchain*", "rust-toolchain.toml"));
        assert!(glob_match("rust-toolchain*", "rust-toolchain"));
        assert!(glob_match("Cargo.toml", "Cargo.toml"));
        assert!(!glob_match("Cargo.toml", "crates/core/Cargo.toml"));
        assert!(glob_match("xtask/**/*.rs", "xtask/src/a/b.rs"));
        assert!(glob_match("xtask/**/*.rs", "xtask/a.rs"));
    }

    #[test]
    fn workspace_closure_includes_transitive_consumers_only() {
        let workspace = fixture_workspace();
        let affected = workspace.closure(&names(&["laneflow-core-lib"]));
        // core 的消费者：mid-lib、leaf-tool、runtime；consumer 经 mid-lib 传递。
        assert_eq!(
            affected,
            names(&[
                "laneflow-core-lib",
                "laneflow-mid-lib",
                "laneflow-leaf-tool",
                "laneflow-runtime",
                "laneflow-consumer"
            ])
        );
        // 叶子工具：无人依赖它，闭包只剩自身。
        assert_eq!(
            workspace.closure(&names(&["laneflow-leaf-tool"])),
            names(&["laneflow-leaf-tool"])
        );
    }

    #[test]
    fn package_of_uses_longest_prefix() {
        let workspace = fixture_workspace();
        assert_eq!(
            workspace.package_of("crates/core/src/lib.rs"),
            Some("laneflow-core-lib")
        );
        assert_eq!(workspace.package_of("crates/other/src/lib.rs"), None);
        assert_eq!(workspace.package_of("crates"), None);
    }

    /// 边界验收用例一（G1 审阅 P2-1）：成员清单变更必须全量，即使该成员是
    /// 反向闭包为空的叶子。
    #[test]
    fn member_manifest_change_forces_full_plan() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["tools/leaf/Cargo.toml"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(plan.full, "叶子成员 Cargo.toml 变更必须扩大为全量");
        assert!(
            plan.reasons
                .iter()
                .any(|reason| reason.contains("feature 合并前提守护"))
        );
    }

    #[test]
    fn unknown_path_forces_full_and_known_docs_do_not() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["docs/design/x.md"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(!plan.full);
        assert!(plan.affected.is_empty(), "纯不适用路径不产生任何种子");
        assert!(plan.tasks.iter().all(|task| task.packages.is_empty()));

        let plan = build_plan(&files(&["VERSION"]), &workspace, &registry(), consistent());
        assert!(plan.full, "未登记路径必须落入未知态扩大为全量");
    }

    #[test]
    fn registry_embed_entry_seeds_owner_and_consumers() {
        let workspace = fixture_workspace();
        // examples/config/cn-urban.toml 在内置注册表里且不属于 fixture 成员：
        // 种子 = 注册表声明的消费者（本 fixture 无所有者）。
        let plan = build_plan(
            &files(&["examples/config/cn-urban.toml"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(!plan.full);
        assert!(plan.seeds.contains("laneflow-urban-generator"));
        assert!(plan.seeds.contains("laneflow-urban-harness"));
    }

    #[test]
    fn unlisted_example_file_is_unknown() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["examples/README.md"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(plan.full, "examples 下未登记嵌入的文件按未知处理");
    }

    #[test]
    fn planner_registry_change_forces_full() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["xtask/data/ci-extra-inputs.toml"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(plan.full, "计划器注册表自身变更必须全量");
    }

    #[test]
    fn motion_trace_tasks_run_on_any_rust_seed_but_not_on_docs_only() {
        let workspace = fixture_workspace();
        // 叶子工具种子：固定配置任务保持执行（与现行 rust 布尔门同语义）。
        let plan = build_plan(
            &files(&["tools/leaf/src/main.rs"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(!plan.full);
        for name in ["clippy-motion-trace", "test-motion-trace"] {
            let task = plan.tasks.iter().find(|task| task.name == name).unwrap();
            assert_eq!(task.packages, ["laneflow-runtime", "laneflow-scenario"]);
        }

        // 纯不适用路径：所有任务（含固定配置）跳过。
        let plan = build_plan(
            &files(&["README.md"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(plan.tasks.iter().all(|task| task.packages.is_empty()));
    }

    #[test]
    fn execution_evidence_tasks_follow_their_respective_consumers() {
        let workspace = Workspace::parse(&cargo_metadata().unwrap()).unwrap();
        let registry = registry();
        let clippy = &["laneflow-runtime", "laneflow-urban-harness"][..];
        let test = &["laneflow-runtime"][..];
        for (path, expected_clippy, expected_test) in [
            ("tools/laneflow-synth-harness/src/main.rs", &[][..], &[][..]),
            ("tools/laneflow-urban-harness/src/main.rs", clippy, &[]),
            ("crates/laneflow-motion-kernel/src/hold.rs", clippy, test),
            ("unknown-ci-input", clippy, test),
        ] {
            let plan = build_plan(&files(&[path]), &workspace, &registry, consistent());
            assert_eq!(plan.full, path == "unknown-ci-input");
            for (name, expected) in [
                ("clippy-execution-evidence", expected_clippy),
                ("test-execution-evidence", expected_test),
            ] {
                let task = plan.tasks.iter().find(|task| task.name == name).unwrap();
                assert_eq!(task.packages, expected, "{path} → {name}");
            }
        }
    }

    #[test]
    fn affected_specialist_tasks_keep_their_complete_package_groups() {
        let workspace = Workspace::parse(&cargo_metadata().unwrap()).unwrap();
        let registry = registry();
        let groups: &[(&str, &[&str])] = &[
            (
                "test-urban-evidence",
                &["laneflow-urban-harness", "laneflow-urban-generator"],
            ),
            ("test-corridor-smoke", &["laneflow-bevy"]),
            (
                "placement-fixtures",
                &[
                    "laneflow-runtime",
                    "laneflow-scenario",
                    "laneflow-junction-generator",
                ],
            ),
            ("test-urban-adapter", &["laneflow-urban-harness"]),
        ];
        for (path, selected) in [
            ("tools/laneflow-synth-harness/src/main.rs", [false; 4]),
            // 只命中证据组的 harness 成员，整组仍须执行。
            (
                "tools/laneflow-urban-harness/src/main.rs",
                [true, false, false, true],
            ),
            // Urban 的 adapter 配置含 Bevy 可选依赖，两项 Evidence 步骤此时均选中。
            ("crates/laneflow-bevy/src/lib.rs", [true, true, false, true]),
            // junction-generator 命中 placement 组，经消费者传播也会命中 Bevy 与 Urban。
            ("tools/laneflow-junction-generator/src/lib.rs", [true; 4]),
            // 包外 fixture 经运行时读取登记同样命中 junction-generator。
            ("examples/data/v0.1-complex-junction.lfca", [true; 4]),
            // #1506 的新数值源同时需要业务专项与仓库范围的 Wire 审计。
            ("crates/laneflow-motion-kernel/src/hold.rs", [true; 4]),
            ("unknown-ci-input", [true; 4]),
        ] {
            let plan = build_plan(&files(&[path]), &workspace, &registry, consistent());
            assert_eq!(plan.full, path == "unknown-ci-input");
            let wire = plan.tasks.iter().find(|t| t.name == "wire-audit").unwrap();
            assert!(!wire.packages.is_empty(), "{path} must keep the wire audit");
            for ((name, packages), run) in groups.iter().zip(selected) {
                let task = plan.tasks.iter().find(|task| task.name == *name).unwrap();
                let expected: &[&str] = if run { packages } else { &[] };
                assert_eq!(task.packages, expected, "{path} → {name}");
            }
        }
    }

    /// 嵌在其他宏实参里的 include 必须被捕获（syn 访问器只到达外层宏，
    /// 嵌套形态在 token 层递归收集），且解析不得越过自身实参边界——
    /// 外层逗号、比较运算、方法调用与同表达式的第二个 include 都不能
    /// 混进 include 的路径。
    #[test]
    fn nested_include_inside_other_macro_is_collected() {
        let cases: &[(&str, Vec<&str>)] = &[
            // 实参后接外层逗号。
            (
                r##"fn f() { assert_eq!(include_str!("../b/input.txt"), "expected"); }"##,
                vec!["../b/input.txt"],
            ),
            // 实参组后是比较运算与外层字面量：旧实现会把 "expected" 拼进路径。
            (
                r##"fn f() { assert!(include_str!("../b/input.txt") == "expected"); }"##,
                vec!["../b/input.txt"],
            ),
            // 实参组后是方法调用：旧实现会把方法名误认作非字面量实参。
            (
                r##"fn f() { assert!(include_str!("../b/input.txt").is_empty()); }"##,
                vec!["../b/input.txt"],
            ),
            // 同一表达式两个 include：旧实现会把第二个吞进第一次的路径。
            (
                r##"fn f() { assert!(include_str!("../b/a.txt") == include_str!("../c/b.txt")); }"##,
                vec!["../b/a.txt", "../c/b.txt"],
            ),
        ];
        for (source, expected) in cases {
            let file = syn::parse_file(source).expect("嵌套形态须可解析");
            let collected: Vec<String> = expected.iter().map(|path| (*path).to_owned()).collect();
            assert_eq!(
                &collect_include_literals(&file),
                &collected,
                "源码：{source}"
            );
        }
    }

    /// 直接形态照常收集；concat!/env! 组合形态记哨兵（非字面量实参）。
    #[test]
    fn direct_include_collected_and_concat_form_is_sentinel() {
        let direct = syn::parse_file(r##"const A: &str = include_str!("../a.txt");"##)
            .expect("直接形态须可解析");
        assert_eq!(
            collect_include_literals(&direct),
            vec!["../a.txt".to_owned()]
        );

        let concat = syn::parse_file(
            r##"const B: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));"##,
        )
        .expect("concat 形态须可解析");
        assert_eq!(
            collect_include_literals(&concat),
            vec![UNPARSED_INCLUDE.to_owned()]
        );
    }

    /// 运行时读取声明与嵌入边合并：v0.1 lfca 的消费者 = 嵌入方 bevy ∪
    /// 运行时读取方 junction-generator（注册表 input 表先例）。用真实
    /// workspace 断言到任务层——默认配置的 test-default 须包含该 generator
    /// （另一配置的 placement 任务不能代替默认配置覆盖，合同 §2）。
    #[test]
    fn declared_runtime_read_reaches_affected_and_default_task() {
        let metadata = cargo_metadata().expect("cargo metadata 须可运行");
        let workspace = Workspace::parse(&metadata).expect("真实 metadata 须可解析");
        let plan = build_plan(
            &files(&["examples/data/v0.1-complex-junction.lfca"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(!plan.full);
        assert!(plan.seeds.contains("laneflow-bevy"), "嵌入边消费者须保留");
        assert!(
            plan.seeds.contains("laneflow-junction-generator"),
            "运行时读取声明消费者须并入"
        );
        assert!(
            plan.affected.contains("laneflow-junction-generator"),
            "受影响集须包含运行时读取方"
        );
        let test_default = plan
            .tasks
            .iter()
            .find(|task| task.name == "test-default")
            .expect("test-default 任务存在");
        assert!(
            test_default
                .packages
                .contains(&"laneflow-junction-generator".to_owned()),
            "默认配置任务须覆盖运行时读取方"
        );
    }

    /// #1459 回归护栏：xtask 数据文件由常驻 Commit message job 校验，通用包
    /// 目录归属不得让它们复活重活。fixture 必须含 xtask 成员并先断言包归属
    /// 前提成立——否则恢复旧的判断顺序（包归属先于不适用）时本测试照样
    /// 通过，起不到回归作用。
    #[test]
    fn xtask_data_files_are_not_applicable_and_yield_empty_tasks() {
        let workspace = fixture_workspace();
        for path in [
            "xtask/data/model-registry.json",
            "xtask/data/harness-registry.json",
            "xtask/data/doc-checks.toml",
        ] {
            assert_eq!(
                workspace.package_of(path),
                Some("xtask"),
                "前提：{path} 在无序规则时会落入包归属（fixture 须含 xtask 成员）"
            );
            let plan = build_plan(&files(&[path]), &workspace, &registry(), consistent());
            assert!(!plan.full, "{path} 应判不适用");
            assert!(plan.seeds.is_empty(), "{path} 不产生种子");
            assert!(
                plan.tasks.iter().all(|task| task.packages.is_empty()),
                "{path} 下所有任务须为空（验证由常驻 job 承载）"
            );
        }
    }

    /// 计划器自身路径的保护边界不受上一条顺序调整影响。
    #[test]
    fn planner_own_paths_still_force_full() {
        let workspace = fixture_workspace();
        for path in [
            "xtask/data/ci-extra-inputs.toml",
            "xtask/Cargo.toml",
            "xtask/src/ci_plan.rs",
        ] {
            let plan = build_plan(&files(&[path]), &workspace, &registry(), consistent());
            assert!(plan.full, "{path} 须保持全量");
        }
    }

    #[test]
    fn bevy_example_gate_follows_gate_set() {
        let workspace = fixture_workspace();
        // 叶子工具不在门集合内：native-example 编译跳过。
        let plan = build_plan(
            &files(&["tools/leaf/src/main.rs"]),
            &workspace,
            &registry(),
            consistent(),
        );
        let gate = plan
            .tasks
            .iter()
            .find(|t| t.name == "compile-bevy-examples")
            .unwrap();
        assert!(gate.packages.is_empty());

        // 改 runtime（真实门集合成员，经闭包进入受影响集）：编译门打开，
        // 执行范围是固定集合 [laneflow-bevy]。
        let plan = build_plan(
            &files(&["crates/runtime/src/lib.rs"]),
            &workspace,
            &registry(),
            consistent(),
        );
        let gate = plan
            .tasks
            .iter()
            .find(|t| t.name == "compile-bevy-examples")
            .unwrap();
        assert_eq!(gate.packages, vec!["laneflow-bevy".to_owned()]);
    }

    /// 一致性对账走计划生成的同一入口：真实仓库的注册表须与全仓
    /// include_str!/include_bytes! 清点逐位相等。本测试是开发期护栏；计划
    /// 的正确性不依赖它是否被执行——计划生成前另有同一对账（合同 §3.2）。
    #[test]
    fn registry_embed_map_matches_live_scan() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let repo_root = repo_root.canonicalize().expect("仓库根可规范化");
        let metadata = cargo_metadata().expect("cargo metadata 须可运行");
        let workspace = Workspace::parse(&metadata).expect("真实 metadata 须可解析");
        let registry = registry();
        match check_registry(&workspace, &repo_root, &registry).expect("全仓嵌入扫描须成功")
        {
            RegistryCheck::Consistent {
                cross_package_edges,
            } => {
                assert_eq!(cross_package_edges, registry.embed.len());
            }
            RegistryCheck::Drift(items) => panic!(
                "注册表与全仓清点不一致：运行 `cargo run --locked -p xtask -- ci-plan --dump-embed-map` 比对并更新 {REGISTRY_PATH}\n{}",
                items.join("\n")
            ),
        }
    }

    /// 可扫描的小仓库（系统临时目录）：三个成员各带一个源文件，`extra`
    /// 按（仓库相对路径，内容）写入源文件，同路径覆盖默认内容；metadata
    /// 与目录一致。
    struct ScanFixture {
        root: PathBuf,
    }

    impl ScanFixture {
        fn new(extra: &[(&str, &str)]) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时钟不早于 Unix 纪元")
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("laneflow-ci-plan-{}-{nanos}", std::process::id()));
            let sources = [
                ("crates/core/src/lib.rs", "pub fn core() {}"),
                ("crates/mid/src/lib.rs", "pub fn mid() {}"),
                ("tools/leaf/src/main.rs", "fn main() {}"),
            ];
            for (path, text) in sources.iter().chain(extra) {
                let file = root.join(path);
                std::fs::create_dir_all(file.parent().expect("源文件有父目录"))
                    .expect("建得出行目录");
                std::fs::write(&file, text).expect("写得出源文件");
            }
            // 扫描固定遍历 crates / tools / xtask 三个目录。
            std::fs::create_dir_all(root.join("xtask")).expect("建得出 xtask 目录");
            Self { root }
        }

        fn workspace(&self) -> Workspace {
            let package = |name: &str, dir: &str, dependencies: serde_json::Value| {
                serde_json::json!({
                    "name": name,
                    "manifest_path": self.root.join(dir),
                    "dependencies": dependencies,
                })
            };
            let metadata = serde_json::json!({
                "workspace_root": self.root,
                "packages": [
                    package("laneflow-core-lib", "crates/core/Cargo.toml", serde_json::json!([])),
                    package("laneflow-mid-lib", "crates/mid/Cargo.toml", serde_json::json!([{"name": "laneflow-core-lib"}])),
                    package("laneflow-leaf-tool", "tools/leaf/Cargo.toml", serde_json::json!([{"name": "laneflow-core-lib"}])),
                ]
            });
            Workspace::parse(&metadata.to_string()).expect("fixture metadata 须可解析")
        }

        fn check(&self, registry_toml: &str) -> Result<RegistryCheck, String> {
            let registry = Registry::parse(registry_toml).expect("fixture 注册表须可解析");
            check_registry(&self.workspace(), &self.root, &registry)
        }
    }

    impl Drop for ScanFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// core 嵌入自己的文件，mid 跨包嵌入同一文件。
    const CROSS_EMBED_SOURCES: &[(&str, &str)] = &[
        (
            "crates/core/src/lib.rs",
            r#"pub const OWN: &str = include_str!("own.txt");"#,
        ),
        (
            "crates/mid/src/lib.rs",
            r#"pub const CORE: &str = include_str!("../../core/src/own.txt");"#,
        ),
    ];

    const CROSS_EMBED_ENTRY: &str = "[[embed]]\npath = \"crates/core/src/own.txt\"\nconsumers = [\"laneflow-core-lib\", \"laneflow-mid-lib\"]\n";

    fn registry_toml(embed: &str, reviewed: &str) -> String {
        format!("schema_version = 1\nreviewed_unparsed_includes = [{reviewed}]\n{embed}")
    }

    #[test]
    fn only_the_reviewed_synth_readme_skips_heavy_tasks() {
        let workspace = Workspace::parse(&cargo_metadata().unwrap()).unwrap();
        let plan = build_plan(
            &files(&["tools/laneflow-synth-harness/README.md"]),
            &workspace,
            &registry(),
            consistent(),
        );
        assert!(!plan.full);
        assert!(plan.tasks.iter().all(|task| task.packages.is_empty()));

        for (path, owner) in [
            ("crates/laneflow-runtime/README.md", "laneflow-runtime"),
            (
                "tools/laneflow-urban-harness/README.md",
                "laneflow-urban-harness",
            ),
        ] {
            assert_eq!(
                classify(path, &workspace, &registry()),
                FileState::Package(owner.to_owned())
            );
        }
    }

    #[test]
    fn same_package_embed_of_exempt_readme_forces_full_plan() {
        let readme = "tools/leaf/README.md";
        let registry = Registry::parse(&registry_toml(
            &format!("[triggers]\nnot_applicable = [\"{readme}\"]"),
            "",
        ))
        .unwrap();
        for attribute in [
            r#"#![doc = include_str!("../README.md")]"#,
            r#"#![cfg_attr(doc, doc = include_str!("../README.md"))]"#,
        ] {
            let fixture = ScanFixture::new(&[("tools/leaf/src/main.rs", attribute)]);
            let workspace = fixture.workspace();
            let check = check_registry(&workspace, &fixture.root, &registry).unwrap();
            assert!(
                matches!(&check, RegistryCheck::Drift(items) if items.iter().any(|item|
                    item.contains(readme) && item.contains("laneflow-leaf-tool")
                )),
                "{check:?}"
            );
            assert!(build_plan(&files(&[readme]), &workspace, &registry, check).full);
        }
    }

    /// 登记与清点一致时对账通过；跨包边一条，值含所有者自身的同包嵌入。
    #[test]
    fn registry_check_passes_when_cross_embed_is_registered() {
        let fixture = ScanFixture::new(CROSS_EMBED_SOURCES);
        assert_eq!(
            fixture.check(&registry_toml(CROSS_EMBED_ENTRY, "")),
            Ok(RegistryCheck::Consistent {
                cross_package_edges: 1
            })
        );
    }

    /// 漏登记的跨包嵌入：对账报差异，带差异的计划不成立——变更文件本身
    /// 只会选出叶子工具的受影响集，也退回全量。先断言「登记后不全量」的
    /// 前提：否则全量可能另有来源，本测试抓不住对账失效。
    #[test]
    fn unregistered_cross_embed_is_drift_and_forces_full_plan() {
        let fixture = ScanFixture::new(CROSS_EMBED_SOURCES);
        let workspace = fixture.workspace();
        let files = files(&["tools/leaf/src/main.rs"]);

        let registered = Registry::parse(&registry_toml(CROSS_EMBED_ENTRY, "")).unwrap();
        let consistent = check_registry(&workspace, &fixture.root, &registered).unwrap();
        let plan = build_plan(&files, &workspace, &registered, consistent);
        assert!(!plan.full, "前提：对账一致时叶子工具变更只是受影响计划");
        assert_eq!(plan.affected, names(&["laneflow-leaf-tool"]));

        let unregistered = Registry::parse(&registry_toml("", "")).unwrap();
        let check = check_registry(&workspace, &fixture.root, &unregistered).unwrap();
        let RegistryCheck::Drift(items) = &check else {
            panic!("漏登记须报差异：{check:?}");
        };
        assert!(
            items.iter().any(
                |item| item.contains("未登记的跨包嵌入：crates/core/src/own.txt")
                    && item.contains("laneflow-mid-lib")
            ),
            "{items:?}"
        );
        let plan = build_plan(&files, &workspace, &unregistered, check);
        assert!(plan.full, "对账不一致时计划须退回全量");
        assert_eq!(
            plan.affected,
            names(&[
                "laneflow-core-lib",
                "laneflow-leaf-tool",
                "laneflow-mid-lib"
            ]),
            "全量 = 全部成员"
        );
    }

    /// 登记失效的两种形态各自成项：登记的嵌入已不存在；嵌入方与登记不符。
    #[test]
    fn stale_entry_and_embedder_mismatch_are_drift() {
        let fixture = ScanFixture::new(CROSS_EMBED_SOURCES);

        let gone =
            "[[embed]]\npath = \"crates/core/src/gone.txt\"\nconsumers = [\"laneflow-mid-lib\"]\n";
        assert!(matches!(
            fixture.check(&registry_toml(gone, "")),
            Ok(RegistryCheck::Drift(ref items)) if items.iter().any(|item| item.contains("登记的跨包嵌入已不存在：crates/core/src/gone.txt"))
        ));

        let partial =
            "[[embed]]\npath = \"crates/core/src/own.txt\"\nconsumers = [\"laneflow-core-lib\"]\n";
        assert!(matches!(
            fixture.check(&registry_toml(partial, "")),
            Ok(RegistryCheck::Drift(ref items)) if items.iter().any(|item| item.contains("嵌入方与登记不符：crates/core/src/own.txt"))
        ));
    }

    /// 非字面量实参的目标无法静态解析：出现位置须逐处在注册表登记为已
    /// 核对，未登记即对账差异；登记后对账通过且不计入跨包边。
    #[test]
    fn unparsed_include_occurrence_must_be_reviewed_in_registry() {
        let fixture = ScanFixture::new(&[(
            "tools/leaf/src/main.rs",
            r#"const LOCK: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));"#,
        )]);
        assert!(matches!(
            fixture.check(&registry_toml("", "")),
            Ok(RegistryCheck::Drift(ref items)) if items.iter().any(|item| item.contains("非字面量 include") && item.contains("tools/leaf/src/main.rs"))
        ));
        assert_eq!(
            fixture.check(&registry_toml("", "\"tools/leaf/src/main.rs\"")),
            Ok(RegistryCheck::Consistent {
                cross_package_edges: 0
            })
        );
    }

    /// 对账做不完（源文件不属于任何成员）是计算失败，不是一份计划。
    #[test]
    fn scan_failure_is_an_error_not_a_plan() {
        let fixture = ScanFixture::new(&[("crates/ghost/src/lib.rs", "pub fn ghost() {}")]);
        let error = fixture
            .check(&registry_toml("", ""))
            .expect_err("不属于成员的源文件须使对账失败");
        assert!(error.contains("不属于任何 workspace 成员"), "{error}");
    }

    /// 对账结论进 summary 与 JSON：一致时写明跨包边数；不一致时模式为
    /// 全量、原因逐条可见，且不再出现「一致」。
    #[test]
    fn registry_check_is_visible_in_summary_and_json() {
        let workspace = fixture_workspace();
        let registry = registry();
        let files = files(&["tools/leaf/src/main.rs"]);

        let plan = build_plan(
            &files,
            &workspace,
            &registry,
            RegistryCheck::Consistent {
                cross_package_edges: 7,
            },
        );
        assert!(plan_summary_text(&plan).contains("注册表对账：一致（跨包嵌入 7 条）"));
        assert!(plan_json_text(&plan).contains("\"status\": \"consistent\""));

        let plan = build_plan(
            &files,
            &workspace,
            &registry,
            RegistryCheck::Drift(vec!["未登记的跨包嵌入：a.txt".to_owned()]),
        );
        let text = plan_summary_text(&plan);
        assert!(text.contains("模式：全量"));
        assert!(text.contains("注册表对账不一致，计划退回全量：未登记的跨包嵌入：a.txt"));
        assert!(!text.contains("注册表对账：一致"));
        assert!(plan_json_text(&plan).contains("\"status\": \"drift\""));
    }

    /// 注册表声明的消费者必须是真实存在的 workspace 成员。
    #[test]
    fn registry_consumers_exist_in_workspace() {
        let metadata = cargo_metadata().expect("cargo metadata 须可运行");
        let workspace = Workspace::parse(&metadata).expect("真实 metadata 须可解析");
        // 用合并后的 inputs（嵌入 ∪ 运行时读取声明）校验，两类消费者都被覆盖。
        for consumers in registry().inputs.values() {
            for consumer in consumers {
                assert!(
                    workspace.roots.contains_key(consumer),
                    "注册表消费者 `{consumer}` 不是 workspace 成员"
                );
            }
        }
    }

    /// 影子模式的事件基线提取：PR / merge_group 各自的载荷字段，其余事件
    /// 与缺字段载荷失败关闭。
    #[test]
    fn event_base_requires_the_event_specific_full_commit_sha() {
        let pr_sha = "a1b2c3d4".repeat(5);
        let pr = serde_json::json!({"pull_request": {"base": {"sha": pr_sha}}});
        assert_eq!(event_base(&pr, "pull_request").unwrap(), pr_sha);
        let merge_sha = "e4f5a6b7".repeat(8);
        let merge_group = serde_json::json!({"merge_group": {"base_sha": merge_sha}});
        assert_eq!(event_base(&merge_group, "merge_group").unwrap(), merge_sha);
        assert!(event_base(&pr, "push").is_err());
        assert!(
            event_base(&serde_json::json!({}), "pull_request").is_err(),
            "缺基线字段须失败关闭"
        );
        for invalid in ["", "HEAD", "a1b2c3d4"] {
            let pr = serde_json::json!({"pull_request": {"base": {"sha": invalid}}});
            assert!(event_base(&pr, "pull_request").is_err());
        }
    }

    /// 影子失败块：显性、说明不阻断、携带错误原文。
    #[test]
    fn shadow_failure_markdown_is_explicit_and_nonblocking() {
        let markdown = shadow_failure_markdown("演示失败");
        assert!(markdown.contains("影子计划失败"));
        assert!(markdown.contains("按现行路径门执行"));
        assert!(markdown.contains("演示失败"));
    }

    /// summary 渲染为纯函数后保持既有内容面（模式、任务行）。
    #[test]
    fn summary_text_lists_mode_and_tasks() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["tools/leaf/src/main.rs"]),
            &workspace,
            &registry(),
            consistent(),
        );
        let text = plan_summary_text(&plan);
        assert!(text.contains("模式：受影响"));
        assert!(text.contains("[执行] clippy-default"));
        assert!(text.contains("tools/leaf/src/main.rs → 包 laneflow-leaf-tool"));
    }

    /// 影子 markdown 的代码围栏必须独立成行：JSON 文本无尾换行，闭合围栏
    /// 前须显式补换行，否则围栏失效、`</details>` 被吞进代码块。
    #[test]
    fn shadow_markdown_fences_stay_on_their_own_lines() {
        let markdown = plan_markdown(
            "影子验证计划",
            "模式：受影响\n",
            "{\n  \"mode\": \"affected\"\n}",
        );
        for line in markdown.lines() {
            if line.contains("```") {
                assert!(
                    line.trim_start().starts_with("```"),
                    "围栏必须独立成行：{line}"
                );
            }
        }
        assert!(markdown.contains("</details>"));
        assert!(markdown.contains("```json"));
    }
}
