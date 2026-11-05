//! 声明式汇总输入（`lf-unified-gate-manifest-1`）的数据模型。
//!
//! manifest 由维护者手工维护：两套负载 × 五轴逐格声明结论、来源身份、覆盖边界、
//! 例外状态与证据锚点。仓库相对路径在 `--repo-root` 下解析；仓库外证据目录用
//! `<evidence-root>/` 前缀占位，运行时经 `--evidence-root` 落地（占位约定见
//! `docs/governance/documentation-policy.md` §3，不写采集机本机布局）。

use serde::{Deserialize, Serialize};

/// manifest 自身的格式身份；随结构不兼容变更而升版。
pub const MANIFEST_FORMAT: &str = "lf-unified-gate-manifest-1";
/// 汇总回执的格式身份（ADR 0033 §4：持久结果带域分隔与版本）。
pub const RECEIPT_FORMAT: &str = "lf-unified-gate-receipt-1";
/// 仓库外证据目录占位前缀。
pub const EVIDENCE_ROOT_PREFIX: &str = "<evidence-root>/";

/// 五轴的固定顺序；receipt 总览矩阵与逐格校验都按此顺序。
pub const AXES: [Axis; 5] = [
    Axis::Behavior,
    Axis::Determinism,
    Axis::Performance,
    Axis::Snapshot,
    Axis::Routing,
];

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Axis {
    Behavior,
    Determinism,
    Performance,
    Snapshot,
    Routing,
}

impl Axis {
    /// 轴标识原文（与 #539 负载 × 验收面矩阵同词）。
    pub fn name(self) -> &'static str {
        match self {
            Axis::Behavior => "behavior",
            Axis::Determinism => "determinism",
            Axis::Performance => "performance",
            Axis::Snapshot => "snapshot",
            Axis::Routing => "routing",
        }
    }
}

/// 逐格结论状态。`uncertified-performance` 是 performance 轴的唯一合法值
/// （维护者 2026-11-05 裁定：v0.10 性能轴结论写为“未认证”，不得写成通过）。
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CellStatus {
    /// 语义对照通过（F 矩阵口径：非产品认证）。
    SemanticPass,
    /// 执行回执通过（gate/回执自身边界内）。
    WitnessPass,
    /// 未认证（性能轴唯一合法状态；Product TBD / Uncertified）。
    UncertifiedPerformance,
    /// 授权豁免（须携带维护者授权与未覆盖范围）。
    AuthorizedExemption,
    /// 已延后（负载级延后时由整行展开）。
    Deferred,
    /// 未完成（试跑允许；正式汇总 `--require-complete` 拒绝）。
    Pending,
}

impl CellStatus {
    /// 回执中的中文短标签。
    pub fn zh(self) -> &'static str {
        match self {
            CellStatus::SemanticPass => "语义对照通过",
            CellStatus::WitnessPass => "执行回执通过",
            CellStatus::UncertifiedPerformance => "未认证",
            CellStatus::AuthorizedExemption => "授权豁免",
            CellStatus::Deferred => "已延后 v0.11",
            CellStatus::Pending => "未完成",
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub title: String,
    /// 本次 manifest 副本的用途说明（试跑/正式）。
    pub mode_note: String,
    /// 本轮要求完成核查的全部 L1 输入身份；`l1_checks` 必须恰好覆盖
    /// （缺失、重复或多挂都失败），防止漏检身份带着「已核查」外观进入结论。
    pub required_l1_identities: Vec<String>,
    pub global: GlobalDeclarations,
    pub loads: Vec<LoadEntry>,
    pub scope_adjudication: Vec<ScopeAdjudication>,
    pub capability_limits: Vec<CapabilityLimit>,
    pub l1_checks: Vec<L1CheckRef>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct GlobalDeclarations {
    /// 单世界扩展声明（#539 验收标准：不以多世界吞吐替代单世界扩展语义）。
    pub single_world_statement: String,
    pub performance: PerformanceDeclaration,
    pub allocation_diagnostics: StatementDeclaration,
    pub checkpoint_verification_cost: StatementDeclaration,
}

/// 性能口径（#1266 回执口径；字段值原样渲染，判定词由汇总器机械校验）。
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct PerformanceDeclaration {
    /// 必须精确等于「未认证」。
    pub verdict: String,
    /// 必须精确等于 `Product TBD / Uncertified`。
    pub product_status: String,
    pub real_budget_failures: u32,
    pub authorized_exceptions: u32,
    pub contract_level_exceedances: Vec<String>,
    /// #1266 处置终态回执链接。
    pub receipt: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct StatementDeclaration {
    pub statement: String,
    pub evidence: Vec<Evidence>,
}

/// 范围裁定清单的一行（#1268 正文 9 项表的声明式副本）。
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ScopeAdjudication {
    pub item: String,
    pub disposition: String,
    pub uncovered: String,
    pub authorization: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CapabilityLimit {
    pub statement: String,
    pub boundary: String,
    pub evidence: Vec<Evidence>,
}

/// L1 拓扑核查记录的挂接：每份正式 L1 输入身份一条 `check-transitions` 记录。
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct L1CheckRef {
    pub identity: String,
    pub record: Evidence,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LoadEntry {
    /// 逐轴声明结论的负载。
    Axes(LoadAxes),
    /// 整体延后的负载（五轴全部按延后渲染，不记为通过）。
    Deferred(LoadDeferred),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LoadAxes {
    pub id: String,
    pub input_identity: String,
    pub cells: Vec<Cell>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LoadDeferred {
    pub id: String,
    pub note: String,
    pub authorization: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub axis: Axis,
    pub status: CellStatus,
    pub conclusion: String,
    pub source_commit: Option<String>,
    pub binary: Option<String>,
    pub input_identity: Option<String>,
    pub coverage: Option<String>,
    pub exception: Option<ExceptionDecl>,
    pub evidence: Vec<Evidence>,
    /// 未完成项（记录缺失或待核对的事实陈述）。
    #[serde(default)]
    pub pending: Vec<String>,
}

/// 例外声明：`disposition` 当前只接受 `authorized-exemption`（不批准例外=没有
/// 其他例外种类；新增种类须先扩本枚举与校验）。
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ExceptionDecl {
    pub disposition: String,
    pub scope: String,
    pub authorization: String,
}

/// 证据锚点。所有 `path` 支持仓库相对路径与 `<evidence-root>/` 前缀占位。
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Evidence {
    /// 仓内文档结论：存在性 + 逐锚点子串命中（锚点可空=仅存在性）。
    RepoDoc {
        path: String,
        #[serde(default)]
        anchors: Vec<String>,
    },
    /// GitHub 记录（Issue/PR/评论）：离线工具只做 URL 形态校验。
    Github { url: String },
    /// LF-SYNTH 矩阵 `summary.json`：解析身份字段并与 `expect` 对账。
    SynthSummaryJson {
        path: String,
        #[serde(default)]
        expect: Option<SynthSummaryExpect>,
    },
    /// 城市负载 `gate.json`：解析身份字段并与 `expect` 对账。
    GateJson {
        path: String,
        #[serde(default)]
        expect: Option<GateJsonExpect>,
    },
    /// #1279 墙钟记录（stdout key=value 捕获）：头部键值必须齐全命中；
    /// 未知键前向容忍（#1267 扩展不破坏消费）。
    RoutingKv {
        path: String,
        #[serde(default)]
        expect: Vec<String>,
    },
    /// `check-transitions` 记录（`lf-l1-topology-check-1`）。
    L1Record {
        path: String,
        #[serde(default)]
        expect: Option<L1RecordExpect>,
    },
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct SynthSummaryExpect {
    pub format: Option<String>,
    pub implementation_commit: Option<String>,
    pub binary_sha256: Option<String>,
    pub complete: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct GateJsonExpect {
    pub status: Option<String>,
    pub commit: Option<String>,
    pub binary_digest: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct L1RecordExpect {
    pub verdict: Option<String>,
}
