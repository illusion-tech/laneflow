//! 统一汇总回执（`lf-unified-gate-receipt-1`）的模型与渲染。
//!
//! 回执是确定性的：同一 manifest 与同一批记录输入产生逐字节相同的回执
//! （不含时间戳与本机路径），便于试跑与正式汇总对拍。机器可读副本为
//! `receipt.json`，人读副本为 `receipt.md`。

use serde::Serialize;

use crate::evidence::EvidenceOutcome;
use crate::manifest::{RECEIPT_FORMAT, ScopeAdjudication};

/// 运行模式：试跑（现有记录，允许未完成项）或正式（#305 在 #1279 完成后运行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Trial,
    Formal,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Trial => "trial",
            Mode::Formal => "formal",
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct Receipt {
    pub format: String,
    pub mode: &'static str,
    pub title: String,
    pub mode_note: String,
    pub overview: Vec<OverviewRow>,
    pub loads: Vec<LoadReceipt>,
    pub scope_adjudication: Vec<ScopeAdjudication>,
    pub capability_limits: Vec<CapabilityLimitReceipt>,
    pub l1_checks: Vec<L1CheckReceipt>,
    pub global: ReceiptGlobal,
    pub pending_items: Vec<PendingItem>,
}

/// 回执侧全局声明：证据已解析为核验产物（不复用 manifest 的原始锚点类型）。
#[derive(Serialize, Debug, Clone)]
pub struct ReceiptGlobal {
    pub single_world_statement: String,
    pub performance: PerformanceReceipt,
    pub allocation_diagnostics: StatementReceipt,
    pub checkpoint_verification_cost: StatementReceipt,
}

#[derive(Serialize, Debug, Clone)]
pub struct PerformanceReceipt {
    pub verdict: String,
    pub product_status: String,
    pub real_budget_failures: u32,
    pub authorized_exceptions: u32,
    pub contract_level_exceedances: Vec<String>,
    pub receipt: String,
    pub evidence: Vec<EvidenceReceipt>,
}

#[derive(Serialize, Debug, Clone)]
pub struct StatementReceipt {
    pub statement: String,
    pub evidence: Vec<EvidenceReceipt>,
}

#[derive(Serialize, Debug, Clone)]
pub struct OverviewRow {
    pub load: String,
    /// 与五轴固定顺序对应的短标签。
    pub cells: [String; 5],
}

#[derive(Serialize, Debug, Clone)]
pub struct LoadReceipt {
    pub id: String,
    pub input_identity: String,
    pub cells: Vec<CellReceipt>,
}

#[derive(Serialize, Debug, Clone)]
pub struct CellReceipt {
    pub axis: String,
    pub status: &'static str,
    pub conclusion: String,
    pub source_commit: Option<String>,
    pub binary: Option<String>,
    pub input_identity: Option<String>,
    pub coverage: Option<String>,
    pub exception: Option<ExceptionReceipt>,
    pub evidence: Vec<EvidenceReceipt>,
    pub pending: Vec<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ExceptionReceipt {
    pub disposition: String,
    pub scope: String,
    pub authorization: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct EvidenceReceipt {
    pub label: String,
    pub detail: String,
}

impl From<EvidenceOutcome> for EvidenceReceipt {
    fn from(outcome: EvidenceOutcome) -> Self {
        EvidenceReceipt {
            label: outcome.label,
            detail: outcome.detail,
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct CapabilityLimitReceipt {
    pub statement: String,
    pub boundary: String,
    pub evidence: Vec<EvidenceReceipt>,
}

#[derive(Serialize, Debug, Clone)]
pub struct L1CheckReceipt {
    pub identity: String,
    pub record: EvidenceReceipt,
}

#[derive(Serialize, Debug, Clone)]
pub struct PendingItem {
    pub load: String,
    pub axis: String,
    pub items: Vec<String>,
}

impl Receipt {
    /// 渲染人读 Markdown 回执。
    ///
    /// # Errors
    /// 不会失败（写入由调用方负责）；保留 `Result` 形态仅为未来扩展。
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", self.title));
        out.push_str(&format!(
            "format `{}` · mode `{}` · 生成自声明式输入（`lf-unified-gate-manifest-1`）：{}\n\n",
            RECEIPT_FORMAT, self.mode, self.mode_note
        ));
        if self.mode == "trial" {
            out.push_str("> **试跑结果不作为 v0.10 结论**（#1268 验收标准：仅验证汇总器消费现有记录；缺失的 #1279 记录与验收提交核对标为未完成）。\n\n");
        } else {
            out.push_str("> 正式汇总：由 #305 在 #1279 完成后运行；performance 轴结论为「未认证」，不构成性能通过。\n\n");
        }

        out.push_str("## 1. 五轴总览\n\n");
        out.push_str(
            "| 负载 \\ 轴 | behavior | determinism | performance | snapshot | routing |\n",
        );
        out.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for row in &self.overview {
            out.push_str(&format!("| {} | {} |\n", row.load, row.cells.join(" | ")));
        }
        out.push('\n');

        out.push_str("## 2. 逐格明细\n\n");
        for load in &self.loads {
            out.push_str(&format!("### {}\n\n", load.id));
            out.push_str(&format!("- 输入身份：{}\n\n", load.input_identity));
            for cell in &load.cells {
                out.push_str(&format!("#### {} · {}\n\n", load.id, cell.axis));
                out.push_str(&format!("- 结论（{}）：{}\n", cell.status, cell.conclusion));
                if let Some(commit) = &cell.source_commit {
                    out.push_str(&format!("- 来源提交：{commit}\n"));
                }
                if let Some(binary) = &cell.binary {
                    out.push_str(&format!("- 被测二进制：{binary}\n"));
                }
                if let Some(identity) = &cell.input_identity {
                    out.push_str(&format!("- 输入身份：{identity}\n"));
                }
                if let Some(coverage) = &cell.coverage {
                    out.push_str(&format!("- 覆盖边界：{coverage}\n"));
                }
                match &cell.exception {
                    Some(exception) => out.push_str(&format!(
                        "- 例外状态：{}（范围：{}；授权：{}）\n",
                        exception.disposition, exception.scope, exception.authorization
                    )),
                    None => out.push_str("- 例外状态：无\n"),
                }
                for evidence in &cell.evidence {
                    out.push_str(&format!(
                        "- 证据：`{}`（{}）\n",
                        evidence.label, evidence.detail
                    ));
                }
                for pending in &cell.pending {
                    out.push_str(&format!("- 未完成：{pending}\n"));
                }
                out.push('\n');
            }
        }

        out.push_str("## 3. 范围裁定清单（维护者 2026-11-05）\n\n");
        out.push_str("| 项目 | 裁定 | 须写入 v0.10 结论的未覆盖范围 | 授权 |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for row in &self.scope_adjudication {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                row.item, row.disposition, row.uncovered, row.authorization
            ));
        }
        out.push('\n');

        out.push_str("## 4. 能力限制登记\n\n");
        for limit in &self.capability_limits {
            out.push_str(&format!(
                "- **{}**\n  - 边界：{}\n",
                limit.statement, limit.boundary
            ));
            for evidence in &limit.evidence {
                out.push_str(&format!(
                    "  - 证据：`{}`（{}）\n",
                    evidence.label, evidence.detail
                ));
            }
        }
        out.push('\n');

        if !self.l1_checks.is_empty() {
            out.push_str("## 5. L1 输入 #1271 拓扑核查\n\n");
            for check in &self.l1_checks {
                out.push_str(&format!(
                    "- {}：`{}`（{}）\n",
                    check.identity, check.record.label, check.record.detail
                ));
            }
            out.push('\n');
        }

        let performance = &self.global.performance;
        out.push_str("## 6. 性能口径（#1266 回执）\n\n");
        out.push_str(&format!(
            "- 性能轴结论：**{}**（{}）——不得写成性能通过。\n",
            performance.verdict, performance.product_status
        ));
        out.push_str(&format!(
            "- 真实预算失败 {} 条；授权例外 {} 条。\n",
            performance.real_budget_failures, performance.authorized_exceptions
        ));
        out.push_str("- 契约级超标（研究观察，正式判定待 P100 冻结环境认证）：\n");
        for item in &performance.contract_level_exceedances {
            out.push_str(&format!("  - {item}\n"));
        }
        out.push_str(&format!("- 处置终态回执：{}\n", performance.receipt));
        for evidence in &performance.evidence {
            out.push_str(&format!(
                "- 证据：`{}`（{}）\n",
                evidence.label, evidence.detail
            ));
        }
        out.push('\n');

        out.push_str("## 7. 诊断口径声明\n\n");
        out.push_str(&format!(
            "- allocation 诊断口径：{}\n",
            self.global.allocation_diagnostics.statement
        ));
        for evidence in &self.global.allocation_diagnostics.evidence {
            out.push_str(&format!(
                "  - 证据：`{}`（{}）\n",
                evidence.label, evidence.detail
            ));
        }
        out.push_str(&format!(
            "- checkpoint 校验成本：{}\n",
            self.global.checkpoint_verification_cost.statement
        ));
        for evidence in &self.global.checkpoint_verification_cost.evidence {
            out.push_str(&format!(
                "  - 证据：`{}`（{}）\n",
                evidence.label, evidence.detail
            ));
        }
        out.push('\n');

        out.push_str("## 8. 边界声明\n\n");
        out.push_str(&format!(
            "- 单世界扩展声明：{}\n",
            self.global.single_world_statement
        ));
        out.push_str("- 延后负载在总览与逐格明细中按「已延后」渲染，不记为通过。\n");
        out.push('\n');

        if !self.pending_items.is_empty() {
            out.push_str("## 9. 未完成项\n\n");
            for item in &self.pending_items {
                out.push_str(&format!(
                    "- {} · {}：{}\n",
                    item.load,
                    item.axis,
                    item.items.join("；")
                ));
            }
            out.push('\n');
        }

        out.push_str("## 10. 来源身份约定\n\n");
        out.push_str("本回执逐格携带来源提交、被测二进制、输入身份、覆盖边界与例外状态；全部身份自既有记录原样携带，本工具不重算摘要（ADR 0033 §3）、不联网核验、不执行测量。\n");
        out
    }
}
