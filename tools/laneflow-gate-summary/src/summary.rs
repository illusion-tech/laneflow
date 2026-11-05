//! 汇编编排：读取声明式 manifest → 校验 → 解析证据 → 产出统一回执。
//!
//! 校验失败关闭：manifest 结构、性能口径判定词、逐格身份完整性与证据锚点
//! 任一不过即失败，不产出半张回执。`--require-complete`（正式汇总）额外
//! 拒绝任何未完成项。

use std::fs;
use std::path::Path;

use crate::evidence::{self, PathContext};
use crate::manifest::{
    AXES, Axis, Cell, CellStatus, Evidence, LoadEntry, MANIFEST_FORMAT, Manifest, RECEIPT_FORMAT,
};
use crate::receipt::{
    CapabilityLimitReceipt, CellReceipt, EvidenceReceipt, ExceptionReceipt, L1CheckReceipt,
    LoadReceipt, Mode, OverviewRow, PendingItem, Receipt,
};
use crate::{Error, invalid};

/// 运行一次汇总，把 `receipt.json` 与 `receipt.md` 写入新建的输出目录。
///
/// # Errors
/// manifest 读取/解析、结构校验、性能口径判定词、证据锚点核验或
/// `require_complete` 拒绝时失败；输出目录已存在时失败。
pub fn run(
    manifest_path: &Path,
    ctx: &PathContext,
    output_dir: &Path,
    mode: Mode,
    require_complete: bool,
) -> Result<Receipt, Error> {
    let text = fs::read_to_string(manifest_path).map_err(|e| {
        invalid(format!(
            "manifest 读取失败 {}：{e}",
            manifest_path.display()
        ))
    })?;
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| invalid(format!("manifest 解析失败：{e}")))?;
    validate(&manifest, mode, require_complete)?;
    let receipt = build_receipt(&manifest, ctx, mode)?;
    if output_dir.exists() {
        return Err(invalid(format!(
            "输出目录必须不存在：{}",
            output_dir.display()
        )));
    }
    fs::create_dir_all(output_dir)?;
    fs::write(
        output_dir.join("receipt.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "format": RECEIPT_FORMAT,
                "mode": mode.name(),
                "receipt": receipt,
            }))?
        ),
    )?;
    fs::write(output_dir.join("receipt.md"), receipt.render_markdown())?;
    Ok(receipt)
}

fn validate(manifest: &Manifest, mode: Mode, require_complete: bool) -> Result<(), Error> {
    if manifest.format != MANIFEST_FORMAT {
        return Err(invalid(format!(
            "manifest 格式不符：{}，期望 {MANIFEST_FORMAT}",
            manifest.format
        )));
    }
    let performance = &manifest.global.performance;
    if performance.verdict != "未认证" {
        return Err(invalid(format!(
            "性能轴结论判定词必须精确为「未认证」，当前为「{}」（维护者 2026-11-05 裁定，见 #305）",
            performance.verdict
        )));
    }
    if performance.product_status != "Product TBD / Uncertified" {
        return Err(invalid(format!(
            "性能产品状态必须精确为 Product TBD / Uncertified，当前为「{}」",
            performance.product_status
        )));
    }
    if manifest.scope_adjudication.is_empty() {
        return Err(invalid(
            "范围裁定清单为空：#1268 交付包含九项裁定表的声明式副本",
        ));
    }
    if manifest.capability_limits.is_empty() {
        return Err(invalid("能力限制登记为空"));
    }
    if manifest.l1_checks.is_empty() {
        return Err(invalid(
            "L1 拓扑核查挂接为空：#1268 须核查 L1 不含 #1271 拓扑",
        ));
    }

    let mut pending: Vec<String> = Vec::new();
    for load in &manifest.loads {
        let LoadEntry::Axes(entry) = load else {
            continue;
        };
        if entry.cells.len() != AXES.len() {
            return Err(invalid(format!(
                "负载 {} 须恰有五轴格，当前 {} 格",
                entry.id,
                entry.cells.len()
            )));
        }
        let mut seen = [false; 5];
        for cell in &entry.cells {
            let Some(index) = AXES.iter().position(|axis| *axis == cell.axis) else {
                return Err(invalid(format!("未知轴：{}", cell.axis.name())));
            };
            if seen[index] {
                return Err(invalid(format!(
                    "负载 {} 的轴 {} 重复",
                    entry.id,
                    cell.axis.name()
                )));
            }
            seen[index] = true;
            validate_cell(&entry.id, cell, mode)?;
            if !cell.pending.is_empty() {
                pending.push(format!("{} · {}", entry.id, cell.axis.name()));
            }
        }
        if seen.contains(&false) {
            return Err(invalid(format!("负载 {} 五轴不齐", entry.id)));
        }
    }

    if require_complete && !pending.is_empty() {
        return Err(invalid(format!(
            "正式汇总（--require-complete）拒绝未完成项：{}",
            pending.join("；")
        )));
    }
    Ok(())
}

fn validate_cell(load: &str, cell: &Cell, mode: Mode) -> Result<(), Error> {
    if cell.axis == Axis::Performance && cell.status != CellStatus::UncertifiedPerformance {
        return Err(invalid(format!(
            "负载 {load} 的 performance 格状态必须为 uncertified-performance（未认证），不得写成通过"
        )));
    }
    let needs_identity = matches!(
        cell.status,
        CellStatus::SemanticPass
            | CellStatus::WitnessPass
            | CellStatus::UncertifiedPerformance
            | CellStatus::AuthorizedExemption
    );
    if needs_identity {
        for (field, value) in [
            ("source_commit", &cell.source_commit),
            ("binary", &cell.binary),
            ("input_identity", &cell.input_identity),
            ("coverage", &cell.coverage),
        ] {
            if value.as_deref().is_none_or(str::is_empty) {
                return Err(invalid(format!(
                    "负载 {load} 的 {} 格缺少 {field}",
                    cell.axis.name()
                )));
            }
        }
        if cell.evidence.is_empty() {
            return Err(invalid(format!(
                "负载 {load} 的 {} 格证据列表为空：通过性结论必须至少挂接一条证据",
                cell.axis.name()
            )));
        }
    }
    match cell.status {
        CellStatus::AuthorizedExemption => {
            let Some(exception) = &cell.exception else {
                return Err(invalid(format!(
                    "负载 {load} 的 {} 格为授权豁免但缺少例外声明",
                    cell.axis.name()
                )));
            };
            if exception.disposition != "authorized-exemption" {
                return Err(invalid(format!(
                    "例外种类当前只接受 authorized-exemption（不批准例外）：{}",
                    exception.disposition
                )));
            }
            if exception.scope.trim().is_empty() || exception.authorization.trim().is_empty() {
                return Err(invalid(format!(
                    "负载 {load} 的 {} 格授权豁免缺少未覆盖范围或维护者授权（豁免替代验证，两者不得为空）",
                    cell.axis.name()
                )));
            }
        }
        CellStatus::Pending if cell.pending.is_empty() => {
            return Err(invalid(format!(
                "负载 {load} 的 {} 格为未完成但缺少未完成项说明",
                cell.axis.name()
            )));
        }
        _ => {}
    }
    if mode == Mode::Formal && cell.status == CellStatus::Pending {
        return Err(invalid(format!(
            "正式汇总拒绝未完成格：{} · {}（补齐记录后重跑）",
            load,
            cell.axis.name()
        )));
    }
    Ok(())
}

fn resolve_all(items: &[Evidence], ctx: &PathContext) -> Result<Vec<EvidenceReceipt>, Error> {
    items
        .iter()
        .map(|item| Ok(evidence::resolve(item, ctx)?.into()))
        .collect()
}

fn build_receipt(manifest: &Manifest, ctx: &PathContext, mode: Mode) -> Result<Receipt, Error> {
    let mut overview = Vec::new();
    let mut loads = Vec::new();
    let mut pending_items = Vec::new();

    for load in &manifest.loads {
        match load {
            LoadEntry::Axes(entry) => {
                let mut cells = Vec::new();
                let mut row = OverviewRow {
                    load: entry.id.clone(),
                    cells: std::array::from_fn(|_| String::new()),
                };
                for cell in &entry.cells {
                    let Some(index) = AXES.iter().position(|axis| *axis == cell.axis) else {
                        return Err(invalid(format!("未知轴：{}", cell.axis.name())));
                    };
                    let evidence = resolve_all(&cell.evidence, ctx)?;
                    row.cells[index] = short_label(cell);
                    if !cell.pending.is_empty() {
                        pending_items.push(PendingItem {
                            load: entry.id.clone(),
                            axis: cell.axis.name().to_owned(),
                            items: cell.pending.clone(),
                        });
                    }
                    cells.push(CellReceipt {
                        axis: cell.axis.name().to_owned(),
                        status: cell.status.zh(),
                        conclusion: cell.conclusion.clone(),
                        source_commit: cell.source_commit.clone(),
                        binary: cell.binary.clone(),
                        input_identity: cell.input_identity.clone(),
                        coverage: cell.coverage.clone(),
                        exception: cell.exception.as_ref().map(|exception| ExceptionReceipt {
                            disposition: exception.disposition.clone(),
                            scope: exception.scope.clone(),
                            authorization: exception.authorization.clone(),
                        }),
                        evidence,
                        pending: cell.pending.clone(),
                    });
                }
                overview.push(row);
                loads.push(LoadReceipt {
                    id: entry.id.clone(),
                    input_identity: entry.input_identity.clone(),
                    cells,
                });
            }
            LoadEntry::Deferred(entry) => {
                let mut row = OverviewRow {
                    load: entry.id.clone(),
                    cells: std::array::from_fn(|_| String::new()),
                };
                let mut cells = Vec::new();
                for (index, axis) in AXES.iter().enumerate() {
                    row.cells[index] = CellStatus::Deferred.zh().to_owned();
                    cells.push(CellReceipt {
                        axis: axis.name().to_owned(),
                        status: CellStatus::Deferred.zh(),
                        conclusion: entry.note.clone(),
                        source_commit: None,
                        binary: None,
                        input_identity: None,
                        coverage: None,
                        exception: None,
                        evidence: Vec::new(),
                        pending: Vec::new(),
                    });
                }
                overview.push(row);
                loads.push(LoadReceipt {
                    id: entry.id.clone(),
                    input_identity: format!("（延后）{}", entry.authorization),
                    cells,
                });
            }
        }
    }

    let capability_limits = manifest
        .capability_limits
        .iter()
        .map(|limit| {
            Ok(CapabilityLimitReceipt {
                statement: limit.statement.clone(),
                boundary: limit.boundary.clone(),
                evidence: resolve_all(&limit.evidence, ctx)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let l1_checks = manifest
        .l1_checks
        .iter()
        .map(|check| {
            let record = evidence::resolve(&check.record, ctx)?;
            let Some(identity) = record.identity.as_deref() else {
                return Err(invalid("L1 核查记录未携带输入身份（解析器缺陷）"));
            };
            if identity != check.identity {
                return Err(invalid(format!(
                    "L1 核查记录身份不符：声明 {} 挂接的记录属于 {}（记录错配）",
                    check.identity, identity
                )));
            }
            Ok(L1CheckReceipt {
                identity: check.identity.clone(),
                record: record.into(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let global = crate::receipt::ReceiptGlobal {
        single_world_statement: manifest.global.single_world_statement.clone(),
        performance: crate::receipt::PerformanceReceipt {
            verdict: manifest.global.performance.verdict.clone(),
            product_status: manifest.global.performance.product_status.clone(),
            real_budget_failures: manifest.global.performance.real_budget_failures,
            authorized_exceptions: manifest.global.performance.authorized_exceptions,
            contract_level_exceedances: manifest
                .global
                .performance
                .contract_level_exceedances
                .clone(),
            receipt: manifest.global.performance.receipt.clone(),
            evidence: resolve_all(&manifest.global.performance.evidence, ctx)?,
        },
        allocation_diagnostics: crate::receipt::StatementReceipt {
            statement: manifest.global.allocation_diagnostics.statement.clone(),
            evidence: resolve_all(&manifest.global.allocation_diagnostics.evidence, ctx)?,
        },
        checkpoint_verification_cost: crate::receipt::StatementReceipt {
            statement: manifest
                .global
                .checkpoint_verification_cost
                .statement
                .clone(),
            evidence: resolve_all(&manifest.global.checkpoint_verification_cost.evidence, ctx)?,
        },
    };

    Ok(Receipt {
        format: RECEIPT_FORMAT.to_owned(),
        mode: mode.name(),
        title: manifest.title.clone(),
        mode_note: manifest.mode_note.clone(),
        overview,
        loads,
        scope_adjudication: manifest.scope_adjudication.clone(),
        capability_limits,
        l1_checks,
        global,
        pending_items,
    })
}

fn short_label(cell: &Cell) -> String {
    let base = cell.status.zh();
    if cell.pending.is_empty() {
        base.to_owned()
    } else {
        format!("{base}（含未完成）")
    }
}
