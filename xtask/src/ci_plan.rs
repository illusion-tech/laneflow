//! CI 受影响验证计划器（#1464 G2 第一批，设计合同 `docs/design/ci-affected-verification-plan.md`）。
//!
//! 纯计算：读变更文件清单、`cargo metadata` 的 workspace 依赖图与额外输入
//! 注册表（`xtask/data/ci-extra-inputs.toml`），产出受影响包集与各验证任务的
//! 执行范围。不执行任何构建；本模块自身不接线 CI 门禁（影子阶段另行交付）。
//!
//! 失败关闭语义（合同 §4）：读取或解析失败由命令以非零退出承载，调用方按
//! 全量处理；文件归类落入「未知」态时直接扩大为完整适用套件；「成功计算后
//! 某任务确实不适用」与「计算失败」严格区分。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

const REGISTRY_PATH: &str = "xtask/data/ci-extra-inputs.toml";
const REGISTRY_TOML: &str = include_str!("../data/ci-extra-inputs.toml");

// ---------------------------------------------------------------------------
// 额外输入注册表
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    schema_version: u32,
    #[serde(default)]
    triggers: TriggerSection,
    #[serde(default)]
    embed: Vec<EmbedEntry>,
    /// 运行时读取声明（`std::fs::read` 等非编译期嵌入的仓库文件依赖）：
    /// 无法自动对账，新增运行时读取仓库文件的测试时人工登记。
    #[serde(default)]
    input: Vec<EmbedEntry>,
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
    /// 跨包嵌入图原始表（仅一致性测试对账用；归类走合并后的 `inputs`）。
    #[allow(dead_code)]
    embed: EmbedMap,
    /// 归类用合并输入图：嵌入边 ∪ 运行时读取声明边。
    inputs: EmbedMap,
}

impl Registry {
    fn parse(text: &str) -> Result<Self, String> {
        let file: RegistryFile =
            toml::from_str(text).map_err(|error| format!("无法解析 {REGISTRY_PATH}：{error}"))?;
        if file.schema_version != 1 {
            return Err(format!("{REGISTRY_PATH} schema_version 须为 1"));
        }
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
            inputs,
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
struct MetadataDoc {
    packages: Vec<PackageDoc>,
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
    // 顺序即优先级：全量触发与成员清单规则先于输入边——成员 manifest 的
    // 跨包读取（如 wire 审计 fixture）不得吞掉 feature 合并前提守护。
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
    if let Some(name) = workspace.package_of(path) {
        return FileState::Package(name.to_owned());
    }
    if let Some(pattern) = registry
        .not_applicable
        .iter()
        .find(|pattern| glob_match(pattern, path))
    {
        return FileState::NotApplicable(pattern.clone());
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
    /// 受影响集与门集合相交非空（或存在任何种子）才执行，执行范围为固定集合。
    Gate {
        gate: &'static [&'static str],
        packages: &'static [&'static str],
    },
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
        packages: TaskPackages::Fixed(&["laneflow-runtime", "laneflow-urban-harness"]),
        note: "execution-evidence feature 的 clippy 护栏，配置身份固定",
    },
    TaskDef {
        name: "test-default",
        packages: TaskPackages::AffectedExclude(&[
            "laneflow-urban-harness",
            "laneflow-urban-generator",
        ]),
        note: "workspace nextest 补集步；corridor_traffic_smoke 目标由专项承载",
    },
    TaskDef {
        name: "test-execution-evidence",
        packages: TaskPackages::Fixed(&["laneflow-runtime"]),
        note: "--lib 精确目标（#1462）",
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
        packages: TaskPackages::Fixed(&["laneflow-urban-harness", "laneflow-urban-generator"]),
        note: "证据仪器与生成器整包重测（#1108 分层）",
    },
    TaskDef {
        name: "test-corridor-smoke",
        packages: TaskPackages::Fixed(&["laneflow-bevy"]),
        note: "--test corridor_traffic_smoke 精确目标（#1462）",
    },
    TaskDef {
        name: "placement-fixtures",
        packages: TaskPackages::Fixed(&[
            "laneflow-runtime",
            "laneflow-scenario",
            "laneflow-junction-generator",
        ]),
        note: "空 feature 副作用护栏：clippy + nextest 全量重跑（#748 P3）",
    },
    TaskDef {
        name: "test-urban-adapter",
        packages: TaskPackages::Fixed(&["laneflow-urban-harness"]),
        note: "adapter feature 的 harness 测试",
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
}

struct TaskPlan {
    name: &'static str,
    packages: Vec<String>,
    note: &'static str,
}

fn build_plan(files: &BTreeSet<String>, workspace: &Workspace, registry: &Registry) -> Plan {
    let mut reasons = Vec::new();
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
    }
}

// ---------------------------------------------------------------------------
// 跨包嵌入扫描（注册表一致性的事实来源）
// ---------------------------------------------------------------------------

/// 对 workspace 成员源码全量清点 `include_str!` / `include_bytes!`，返回
/// （仓库相对路径 → 嵌入方包名 集合，含同包与跨包由调用方过滤；
/// 非字面量实参的源文件清单）。非字面量实参（如
/// `concat!(env!("CARGO_MANIFEST_DIR"), …)`）无法静态解析，不入图、列为
/// 人工核对项；已知实例是 lust-converter 嵌入根 `Cargo.lock`——该路径已是
/// 全量触发器，归类正确性不受影响。扫描失败即失败关闭。
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
    unparsed.sort();
    unparsed.dedup();
    Ok((map, unparsed))
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
    struct Visitor {
        targets: Vec<String>,
    }
    impl syn::visit::Visit<'_> for Visitor {
        fn visit_macro(&mut self, mac: &syn::Macro) {
            let ident = mac.path.segments.last().map(|s| s.ident.to_string());
            if ident.as_deref() == Some("include_str") || ident.as_deref() == Some("include_bytes")
            {
                self.targets.push(direct_include_argument(&mac.tokens));
            }
            scan_nested_includes(&mac.tokens, &mut self.targets);
        }
    }
    let mut visitor = Visitor {
        targets: Vec::new(),
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
/// 调用（标识符后随可选 `!` 与一个括号组），并继续深入所有组。
fn scan_nested_includes(tokens: &proc_macro2::TokenStream, targets: &mut Vec<String>) {
    let tokens: Vec<proc_macro2::TokenTree> = tokens.clone().into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            proc_macro2::TokenTree::Group(group) => {
                scan_nested_includes(&group.stream(), targets);
                index += 1;
            }
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if name != "include_str" && name != "include_bytes" {
                    index += 1;
                    continue;
                }
                // 标识符 → 可选 `!` → 实参（括号组或裸字面量序列）。
                let mut cursor = index + 1;
                if matches!(tokens.get(cursor), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '!')
                {
                    cursor += 1;
                }
                let mut literal = String::new();
                let mut non_literal = false;
                while let Some(token) = tokens.get(cursor) {
                    match token {
                        proc_macro2::TokenTree::Literal(lit) => {
                            push_literal_text(lit, &mut literal);
                        }
                        proc_macro2::TokenTree::Group(group) => {
                            let inner: Vec<_> = group.stream().into_iter().collect();
                            for token in inner {
                                match token {
                                    proc_macro2::TokenTree::Literal(lit) => {
                                        push_literal_text(&lit, &mut literal)
                                    }
                                    proc_macro2::TokenTree::Ident(_) => non_literal = true,
                                    _ => {}
                                }
                            }
                        }
                        proc_macro2::TokenTree::Ident(_) => non_literal = true,
                        proc_macro2::TokenTree::Punct(punct) if punct.as_char() != ',' => {}
                        _ => break,
                    }
                    cursor += 1;
                }
                targets.push(if literal.is_empty() || non_literal {
                    UNPARSED_INCLUDE.to_owned()
                } else {
                    literal
                });
                index = cursor;
            }
            _ => index += 1,
        }
    }
}

fn push_literal_text(literal: &proc_macro2::Literal, out: &mut String) {
    let text = literal.to_string();
    if let Some(stripped) = text.strip_prefix('"').and_then(|it| it.strip_suffix('"')) {
        out.push_str(stripped);
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
    let mut rev_range: Option<&str> = None;
    let mut files_path: Option<&str> = None;
    let mut metadata_path: Option<&str> = None;
    let mut dump_embed_map = false;
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
            "--format" => {
                index += 1;
                format = args.get(index).ok_or("ci-plan: --format 缺少值")?;
            }
            other => return Err(format!("ci-plan: 未知参数 {other}")),
        }
        index += 1;
    }

    let metadata = match metadata_path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("ci-plan: 无法读取 metadata `{path}`：{error}"))?,
        None => cargo_metadata()?,
    };
    let workspace = Workspace::parse(&metadata)?;
    let registry = Registry::parse(REGISTRY_TOML)?;
    let repo_root = std::env::current_dir()
        .map_err(|error| format!("ci-plan: 无法确定工作目录：{error}"))?
        .canonicalize()
        .map_err(|error| format!("ci-plan: 无法规范化工作目录：{error}"))?;

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

    let plan = build_plan(&files, &workspace, &registry);
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
    let text = String::from_utf8_lossy(&output.stdout);
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

fn print_json(plan: &Plan) {
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
        "reasons": plan.reasons,
        "seed_packages": plan.seeds,
        "affected_packages": plan.affected,
        "tasks": tasks,
        "classification": classification,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("计划序列化不可失败")
    );
}

fn print_summary(plan: &Plan) {
    if plan.full {
        println!("模式：全量（完整适用套件）");
    } else {
        println!("模式：受影响");
    }
    for reason in &plan.reasons {
        println!("  {reason}");
    }
    println!("种子包：{}", join(&plan.seeds));
    println!("受影响包：{}", join(&plan.affected));
    println!("任务：");
    for task in &plan.tasks {
        if task.packages.is_empty() {
            println!("  [跳过] {}（{}）", task.name, task.note);
        } else {
            println!(
                "  [执行] {} → {}（{}）",
                task.name,
                task.packages.join(","),
                task.note
            );
        }
    }
    println!("归类：");
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
        println!("  {path} → {kind}");
    }
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
            {"name": "laneflow-consumer", "manifest_path": "/repo/crates/consumer/Cargo.toml", "dependencies": [{"name": "laneflow-mid-lib"}]}
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
        let plan = build_plan(&files(&["tools/leaf/Cargo.toml"]), &workspace, &registry());
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
        let plan = build_plan(&files(&["docs/design/x.md"]), &workspace, &registry());
        assert!(!plan.full);
        assert!(plan.affected.is_empty(), "纯不适用路径不产生任何种子");
        assert!(plan.tasks.iter().all(|task| task.packages.is_empty()));

        let plan = build_plan(&files(&["VERSION"]), &workspace, &registry());
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
        );
        assert!(!plan.full);
        assert!(plan.seeds.contains("laneflow-urban-generator"));
        assert!(plan.seeds.contains("laneflow-urban-harness"));
    }

    #[test]
    fn unlisted_example_file_is_unknown() {
        let workspace = fixture_workspace();
        let plan = build_plan(&files(&["examples/README.md"]), &workspace, &registry());
        assert!(plan.full, "examples 下未登记嵌入的文件按未知处理");
    }

    #[test]
    fn planner_registry_change_forces_full() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["xtask/data/ci-extra-inputs.toml"]),
            &workspace,
            &registry(),
        );
        assert!(plan.full, "计划器注册表自身变更必须全量");
    }

    #[test]
    fn fixed_tasks_run_on_any_rust_seed_but_not_on_docs_only() {
        let workspace = fixture_workspace();
        // 叶子工具种子：固定配置任务保持执行（与现行 rust 布尔门同语义）。
        let plan = build_plan(&files(&["tools/leaf/src/main.rs"]), &workspace, &registry());
        assert!(!plan.full);
        let placement = plan
            .tasks
            .iter()
            .find(|t| t.name == "placement-fixtures")
            .unwrap();
        assert!(!placement.packages.is_empty());

        // 纯不适用路径：所有任务（含固定配置）跳过。
        let plan = build_plan(&files(&["README.md"]), &workspace, &registry());
        let placement = plan
            .tasks
            .iter()
            .find(|t| t.name == "placement-fixtures")
            .unwrap();
        assert!(placement.packages.is_empty());
    }

    /// 嵌在其他宏实参里的 include 必须被捕获（syn 访问器只到达外层宏，
    /// 嵌套形态在 token 层递归收集）。
    #[test]
    fn nested_include_inside_other_macro_is_collected() {
        let file = syn::parse_file(
            r##"fn f() { assert_eq!(include_str!("../b/input.txt"), "expected"); }"##,
        )
        .expect("嵌套形态须可解析");
        assert_eq!(
            collect_include_literals(&file),
            vec!["../b/input.txt".to_owned()]
        );
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

    /// 运行时读取声明与嵌入边合并成种子：v0.1 lfca 的消费者 = 嵌入方 bevy
    /// ∪ 运行时读取方 junction-generator（注册表 input 表先例）。
    #[test]
    fn declared_runtime_read_merges_with_embed_consumers() {
        let workspace = fixture_workspace();
        let plan = build_plan(
            &files(&["examples/data/v0.1-complex-junction.lfca"]),
            &workspace,
            &registry(),
        );
        assert!(!plan.full);
        assert!(plan.seeds.contains("laneflow-bevy"), "嵌入边消费者须保留");
        assert!(
            plan.seeds.contains("laneflow-junction-generator"),
            "运行时读取声明消费者须并入"
        );
    }

    #[test]
    fn bevy_example_gate_follows_gate_set() {
        let workspace = fixture_workspace();
        // 叶子工具不在门集合内：native-example 编译跳过。
        let plan = build_plan(&files(&["tools/leaf/src/main.rs"]), &workspace, &registry());
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
        );
        let gate = plan
            .tasks
            .iter()
            .find(|t| t.name == "compile-bevy-examples")
            .unwrap();
        assert_eq!(gate.packages, vec!["laneflow-bevy".to_owned()]);
    }

    /// 一致性对账：注册表 embed 图必须与全仓 include_str!/include_bytes!
    /// 清点出的跨包嵌入逐位相等——新增跨包嵌入时本测试抓红，先补注册表。
    #[test]
    fn registry_embed_map_matches_live_scan() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let repo_root = repo_root.canonicalize().expect("仓库根可规范化");
        let metadata = cargo_metadata().expect("cargo metadata 须可运行");
        let workspace = Workspace::parse(&metadata).expect("真实 metadata 须可解析");
        let (scanned, unparsed) = scan_embeds(&workspace, &repo_root).expect("全仓嵌入扫描须成功");
        assert_eq!(
            unparsed,
            vec!["tools/laneflow-lust-converter/src/output/pipeline.rs"],
            "非字面量 include 实参清单变化须同步本断言与注册表说明"
        );

        // 只保留跨包嵌入：目标不属于任何成员，或所有者 ≠ 嵌入方。
        let cross: BTreeMap<String, BTreeSet<String>> = scanned
            .into_iter()
            .filter(|(target, embedders)| {
                embedders.iter().any(|embedder| {
                    workspace
                        .package_of(target)
                        .map(|owner| owner != embedder)
                        .unwrap_or(true)
                })
            })
            .collect();
        assert_eq!(
            cross,
            registry().embed,
            "跨包嵌入图与注册表不一致：运行 `cargo run --locked -p xtask -- ci-plan --dump-embed-map` 比对并更新 {REGISTRY_PATH}"
        );
    }

    /// 注册表声明的消费者必须是真实存在的 workspace 成员。
    #[test]
    fn registry_consumers_exist_in_workspace() {
        let metadata = cargo_metadata().expect("cargo metadata 须可运行");
        let workspace = Workspace::parse(&metadata).expect("真实 metadata 须可解析");
        for consumers in registry().embed.values() {
            for consumer in consumers {
                assert!(
                    workspace.roots.contains_key(consumer),
                    "注册表消费者 `{consumer}` 不是 workspace 成员"
                );
            }
        }
    }
}
