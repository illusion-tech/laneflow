//! 几何修复：SUMO 折线中心线 → Road Editing 曲线程序。
//!
//! 依据 #253 G1 修订（几何修复策略，issuecomment-5853823447）：维持 Balanced2Deg
//! 不变，修复全部落在 converter 发射层，compiler / ADR 不动：
//!
//! 1. **平滑重拟合**：软折点（弦间方向变化 ≤ 30°，SUMO 粗采样伪影）构成的运行段
//!    用 Catmull-Rom 式切向构造 C1 连续参考曲线，再以 ≤1.2°/弦 + 矢高 ≤1cm +
//!    弦长下限 0.105m 采样为致密 Line 链发射，发射时按 f32 量化后的真实弦
//!    逐段预检 HIR 退化段下限（>0.1m）与 weld 全角预算（≤1.95° < 2°）；
//! 2. **边界切向对齐**：maneuver 路径完整边序列中相邻边的端点切向钳制到边界
//!    两侧原始弦方向的角均值；扇出端（被多条路径共享，如入口边末端）取全部
//!    候选的角均值，独占端跟随对端钳制值，保证每一边界对跳变 ≈ 0；
//! 3. **硬角倒圆**：弦间方向变化 > 30° 的真实折角（OSM 源几何）按有界半径
//!    r ≤ 5.0 m 倒圆，回切不超过相邻弦长的 45%，偏离原折点 ≤ 5.0 m。
//!
//! 发射形态说明（G1 修订的实现级修正）：初版按 G1 字面以三次 Bezier 段发射，
//! 但 compiler 对 Bezier 的 candidate 验收含"端点切向 vs 弦"半角检查
//! （Balanced2Deg 半角 1°，对齐参考 Smooth1Deg 半角 0.5°），细分端点经 f32
//! 量化——在 |坐标| ~7000 的 LuST 全网上 ulp ≈ 0.49mm，半径 < ~10 m 的弯道
//! 验收窗口不存在，必然 ApproximationNotConverged。Line 段恒在 depth 0 验收、
//! 段间只做全角 weld 检查，致密折线发射在同样语义下稳定通过。
//!
//! 修复只动切向与内部形状：所有原始端点位置保持不变，maneuver 边界的位置
//! 连续性（DiscontinuousJoin）不受影响。

use std::collections::{HashMap, HashSet};

use laneflow_compiler::road_editing as re;

use crate::{
    Error, Result,
    output::model::{SpatialEdge, TrafficPackage},
    source::{LUST_COMMIT, PINNED_SOURCE_FILES},
    sumo::SUMO_ID_PREFIX,
};

// ---------------------------------------------------------------------------
// 发射层不可行诊断清单（验收重划后的正式交付物，见 #253 G1 补充记录）
// ---------------------------------------------------------------------------

/// 单条 lane 的发射层不可行机制分类。
///
/// 分类规则（与清单渲染和验收测试锁定值一致）：失败 span 的两端切向均为
/// 倒圆预设 → 硬折角倒圆；任一端为边界钳制 → 边界钳制相关；否则 → 内部曲率。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfeasibilityMechanism {
    HardCornerFillet,
    BoundaryClamp,
    InteriorCurvature,
}

impl InfeasibilityMechanism {
    pub fn label(self) -> &'static str {
        match self {
            InfeasibilityMechanism::HardCornerFillet => "硬折角倒圆",
            InfeasibilityMechanism::BoundaryClamp => "边界钳制相关",
            InfeasibilityMechanism::InteriorCurvature => "纯内部 CR",
        }
    }
}

/// 发射预算裁决（#253 G1 六条件之重新验收；R9 审计拆分）：
/// - `SamplerExhausted`：等 t 与等切向角划分均失败（R6 回溯轮取尽）。
/// - `PreCheckRejected`：质量目标预检拒绝——`sample_cubic` 两处
///   "curvature is infeasible" 文案均由 0.105 m **采样质量目标**（非硬下限
///   0.1 m）驱动的候选搜索前预检产生，不构成硬预算无解证明（复审数值反例：
///   预检拒绝的曲线两段划分后满足全部硬预算，见
///   `quality_floor_precheck_is_not_proven_conflict` 回归）。
/// - `ProvenBudgetConflict`：真必要条件证明。**审计结论：当前发射层无此
///   发射点**（两处候选均为质量目标预检），枚举保留以备未来真证明路径。
/// - `NotBudget`：非预算类失败。
///
/// 与「已归一化（stub 删焊处置记录）」「未支持-未评估（结构性 unsupported
/// fail-closed 于转换前，不进清单）」合为处置口径。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetOutcome {
    NotBudget,
    SamplerExhausted,
    PreCheckRejected,
    ProvenBudgetConflict,
}

impl BudgetOutcome {
    pub fn label(self) -> &'static str {
        match self {
            BudgetOutcome::NotBudget => "非预算裁决",
            BudgetOutcome::SamplerExhausted => "采样候选耗尽",
            BudgetOutcome::PreCheckRejected => "预算预检拒绝",
            BudgetOutcome::ProvenBudgetConflict => "已证预算冲突",
        }
    }
}

/// 一条不可行 lane 的确定性诊断记录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InfeasibilityDiagnosis {
    pub lane_id: String,
    /// SUMO 内车道（`sumo:` 前缀后的原始 id 以 `:` 开头）。
    pub is_internal: bool,
    /// 内车道路口 id（`sumo:-<j>_...` → `-<j>`）；非内车道或 id 无法解析为
    /// 整数路口（off-ramp 节点簇）时为 `None`。
    pub junction: Option<String>,
    pub mechanism: InfeasibilityMechanism,
    /// 发射预算裁决（采样候选耗尽 / 已证预算冲突 / 非预算类）。
    pub budget_outcome: BudgetOutcome,
    /// 单行规范诊断：span j/n、弦长、单片/均转角、坐标、start/finish 切向来源。
    pub entry: String,
}

/// 由 lane id 与发射层规范错误串构造诊断记录（纯函数；收集由调用方持有，
/// 无跨转换全局状态）。
/// 由调用方按权威元数据（`SumoLane::function_internal`）解析好的
/// internal 标记（#253 Q6：id 前缀只是 LuST 命名习惯）。
pub(crate) fn classify_infeasible(
    lane_id: &str,
    entry: String,
    is_internal: bool,
) -> InfeasibilityDiagnosis {
    let junction = lane_id
        .strip_prefix(SUMO_ID_PREFIX)
        .filter(|rest| rest.starts_with(':'))
        .and_then(|rest| rest[1..].split('_').next())
        .filter(|head| head.parse::<i64>().is_ok())
        .map(str::to_owned);
    let mechanism = if entry.matches("fillet arc tangent (prescribed)").count() >= 2 {
        InfeasibilityMechanism::HardCornerFillet
    } else if entry.contains("boundary clamp") {
        InfeasibilityMechanism::BoundaryClamp
    } else {
        InfeasibilityMechanism::InteriorCurvature
    };
    let budget_outcome = if entry.contains("sampling exhausted") {
        BudgetOutcome::SamplerExhausted
    } else if entry.contains("curvature is infeasible") {
        // R9 审计：该文案只有质量目标预检一个来源（见 sample_cubic 两处
        // 注释），不是硬预算无解证明。
        BudgetOutcome::PreCheckRejected
    } else {
        BudgetOutcome::NotBudget
    };
    InfeasibilityDiagnosis {
        lane_id: lane_id.to_owned(),
        is_internal,
        junction,
        mechanism,
        budget_outcome,
        entry,
    }
}

/// 诊断模式兜底：为不可发射的 lane 构造首末点直连的占位程序，保持 LaneEdge
/// 全数在场、下游计数不级联。占位曲线不进入 compiler 验收（诊断模式下
/// compiler 阶段被跳过）；预检算术本身未被削弱，失败以上述诊断记录为准。
pub(crate) fn survey_fallback_program(points: &[Vec3]) -> Result<re::RoadEditingCurveProgram> {
    let first = points.first().copied().unwrap_or([0.0; 3]);
    let last = points.last().copied().unwrap_or(first);
    let chord = sub(last, first);
    let end = if norm(chord) > SAMPLE_MIN_CHORD_METERS {
        last
    } else {
        add(first, [DEGENERATE_SEGMENT_METERS, 0.0, 0.0])
    };
    Ok(re::RoadEditingCurveProgram::try_new(
        point3(first)?,
        vec![re::RoadEditingCurveSegment::line(point3(end)?)],
    )?)
}

/// lane 是否为 SUMO 内车道（`sumo:` 前缀后的原始 id 以 `:` 开头）。
/// 从发射层规范错误串解析诊断字段。格式由本模块的错误构造唯一产生；
/// 任何字段缺失时落 `"?"`，不影响清单的确定性。
fn parse_diagnosis_fields(entry: &str) -> (String, String, String, String, String, String) {
    fn between<'a>(text: &'a str, pre: &str, post: &str) -> Option<&'a str> {
        text.split_once(pre)
            .and_then(|(_, tail)| tail.split_once(post))
            .map(|(mid, _)| mid)
    }
    let span = between(entry, "span ", " (chord").unwrap_or("?").to_owned();
    let chord = between(entry, "(chord ", " m)").unwrap_or("?").to_owned();
    let turn = entry
        .split_once("each turn ")
        .and_then(|(_, tail)| between(tail, "", " deg"))
        .or_else(|| {
            entry
                .split_once("turns ")
                .and_then(|(_, tail)| between(tail, "", " deg >"))
        })
        .unwrap_or("?")
        .to_owned();
    let coord = between(entry, "at (", ");").unwrap_or("?").to_owned();
    let start = between(entry, "start tangent: ", "; finish tangent:")
        .unwrap_or("?")
        .to_owned();
    let finish = entry
        .split_once("finish tangent: ")
        .map(|(_, tail)| tail.trim().to_owned())
        .unwrap_or_else(|| "?".to_owned());
    (span, chord, turn, coord, start, finish)
}

/// 诊断清单的来源声明：实际参与转换的输入 net XML 字节摘要与独立校验状态。
///
/// 摘要对**实际转换所依据的字节**求值（构造点即 xml 入口），清单不可能声称
/// 与输入不符的基线；`verified` 仅当输入经 `verify_source`（checkout revision +
/// pinned digest）走过时为 true。
///
/// 字段私有 + 受控构造器（#253 R2）：外部调用方无法自行构造
/// `verified = true` 的实例——已验证声明只能来自 `prepare_verified_lust_inputs`
/// 或 crate 内 pipeline 的 verify-source 路径，「调用方自我声明已验证」在
/// 类型层面不可能。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportSource {
    /// 实际转换输入的 SHA-256（`sha256:` 前缀十六进制）；入口未持有原始字节
    /// 时为 `None`（渲染标注「未知」）。
    net_digest: Option<String>,
    /// 输入是否经 `verify_source` 独立校验。
    verified: bool,
}

impl ReportSource {
    /// 未持有原始字节的入口（SumoNetwork 结构化入口）的诚实缺省：不声称已校验。
    /// 当前仅单测/fixture 消费；生产 pipeline 的来源声明走 `verified`。
    #[cfg(test)]
    pub fn unverified_unknown() -> Self {
        Self {
            net_digest: None,
            verified: false,
        }
    }

    /// XML 入口的诊断来源声明：实际输入字节摘要 + 未执行独立校验。
    /// 当前仅验收套件（迁入 crate 内的组合入口 helper）消费。
    #[cfg(test)]
    pub(crate) fn xml_unverified(net_digest: String) -> Self {
        Self {
            net_digest: Some(net_digest),
            verified: false,
        }
    }

    /// verify-source 走过的已验证声明（crate 内：pipeline 与
    /// `prepare_verified_lust_inputs`）。
    pub(crate) fn verified(net_digest: String) -> Self {
        Self {
            net_digest: Some(net_digest),
            verified: true,
        }
    }

    /// 实际转换输入的 SHA-256（`sha256:` 前缀十六进制），未持有时为 `None`。
    pub fn net_digest(&self) -> Option<&str> {
        self.net_digest.as_deref()
    }

    /// 输入是否经 `verify_source`（checkout revision + pinned digest）独立校验。
    pub fn is_verified(&self) -> bool {
        self.verified
    }
}

/// pinned 基线参照（`scenario/lust.net.xml`）；清单头以此标注比对目标，
/// 不声称输入即基线。
fn pinned_net_reference() -> (&'static str, &'static str) {
    let pinned = PINNED_SOURCE_FILES
        .iter()
        .find(|file| file.relative_path == "scenario/lust.net.xml")
        .expect("pinned lust.net.xml entry");
    (LUST_COMMIT, pinned.sha256_hex)
}

/// 确定性诊断清单（验收重划后的正式交付物，见 #253 G1 补充记录）：
/// 同一输入下两次运行的 `rendered` 逐字节一致。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InfeasibilityReport {
    pub entries: Vec<InfeasibilityDiagnosis>,
    /// 逐字节稳定的 Markdown 渲染（诊断清单交付格式）。
    pub rendered: String,
}

impl InfeasibilityReport {
    /// 普查锁定计数口径（#253 验收锚点）：当前仅验收套件消费。
    #[cfg(test)]
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// 普查锁定计数口径：当前仅验收套件消费。
    #[cfg(test)]
    pub fn internal_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_internal).count()
    }

    /// 普查锁定计数口径：当前仅验收套件消费。
    #[cfg(test)]
    pub fn external_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_internal).count()
    }

    /// 普查锁定计数口径：当前仅验收套件消费。
    #[cfg(test)]
    pub fn outcome_count(&self, outcome: BudgetOutcome) -> usize {
        self.entries
            .iter()
            .filter(|e| e.budget_outcome == outcome)
            .count()
    }

    /// 内车道子集的机制计数（普查锁定的口径；authored 边不计入）。
    /// 当前仅验收套件消费。
    #[cfg(test)]
    pub fn internal_mechanism_count(&self, mechanism: InfeasibilityMechanism) -> usize {
        self.entries
            .iter()
            .filter(|e| e.is_internal && e.mechanism == mechanism)
            .count()
    }

    /// 普查锁定计数口径：当前仅验收套件消费。
    #[cfg(test)]
    pub fn junction_count(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|e| e.junction.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// 由诊断记录渲染确定性 Markdown 清单；`weld_records` 为 stub 删焊处置
    /// 记录（已归一化类），渲染为附录随清单落出（G1 可追溯）。
    pub fn render(
        entries: Vec<InfeasibilityDiagnosis>,
        source: ReportSource,
        weld_records: &[crate::convert::junction::StubWeldRecord],
    ) -> Self {
        let internal = entries.iter().filter(|e| e.is_internal).count();
        let external = entries.len() - internal;
        let internal_mechanism = |m: InfeasibilityMechanism| {
            entries
                .iter()
                .filter(|e| e.is_internal && e.mechanism == m)
                .count()
        };
        let hard = internal_mechanism(InfeasibilityMechanism::HardCornerFillet);
        let clamp = internal_mechanism(InfeasibilityMechanism::BoundaryClamp);
        let cr = internal_mechanism(InfeasibilityMechanism::InteriorCurvature);
        let junctions = entries
            .iter()
            .filter_map(|e| e.junction.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let mut out = String::new();
        out.push_str("# LuST 发射层不可行诊断清单\n\n");
        let (pinned_commit, pinned_digest) = pinned_net_reference();
        out.push_str(&format!(
            "- 基线参照：pinned LuST v2.0 @ `{pinned_commit}`；`scenario/lust.net.xml`

              pinned digest `sha256:{pinned_digest}`。
"
        ));
        match source.net_digest() {
            Some(digest) => out.push_str(&format!(
                "- 本次转换输入 net XML 摘要：`{digest}`（对实际参与转换的字节求值）。
"
            )),
            None => out.push_str(
                "- 本次转换输入 net XML 摘要：未知（入口未持有原始字节）。
",
            ),
        }
        match source.net_digest() {
            Some(digest) => {
                let pinned = format!("sha256:{pinned_digest}");
                if digest == pinned {
                    out.push_str(
                        "- 与 pinned 比对：一致（输入为 pinned 基线字节）。
",
                    );
                } else {
                    out.push_str(
                        "- 与 pinned 比对：不一致（输入不是 pinned 基线字节）。
",
                    );
                }
            }
            None => out.push_str(
                "- 与 pinned 比对：未知（无输入摘要）。
",
            ),
        }
        if source.is_verified() {
            out.push_str(
                "- 来源校验：verify-source 已通过（checkout revision + pinned digest）。
",
            );
        } else {
            out.push_str(
                "- 来源校验：未执行独立 verify-source；输入摘要针对实际转换的字节，\
                 verify-source 校验磁盘 checkout（见 `laneflow_lust_converter::verify_source`），\
                 两者互补，未校验输入不声称已校验。
",
            );
        }
        out.push_str("- 语义：发射层（CR 平滑 + 边界钳制 + 倒圆 + 采样）在本套验收常数\n");
        out.push_str("  （Balanced2Deg 全角 2°、HIR 退化段 0.1 m、发射弦长下限 0.105 m、\n");
        out.push_str("  f32 端点量化）下无法给出可发射几何的 lane 全集；逐条含 span、弦长、\n");
        out.push_str("  单片/均转角、坐标、切向来源与机制分类。\n");
        out.push_str(
            "- 生成：converter 诊断模式（`TopologyConvertOptions::emit_infeasibility_report`）；\n",
        );
        out.push_str("  同一基线下两次运行本清单逐字节一致。\n\n");
        out.push_str("## 总量\n\n");
        out.push_str("| 类别 | 不可行 | 基数 | 占比 |\n| --- | ---: | ---: | ---: |\n");
        out.push_str(&format!(
            "| SUMO 内车道（自动生成） | {internal} | 15,953 | {:.1}% |\n",
            internal as f64 / 159.53
        ));
        out.push_str(&format!(
            "| authored 边（external） | {external} | 8,622 | {:.1}% |\n",
            external as f64 / 86.22
        ));
        out.push_str(&format!(
            "| **合计** | **{}** | 24,575 | {:.1}% |\n\n",
            entries.len(),
            entries.len() as f64 / 245.75
        ));
        let exhausted = entries
            .iter()
            .filter(|e| e.budget_outcome == BudgetOutcome::SamplerExhausted)
            .count();
        let proven = entries
            .iter()
            .filter(|e| e.budget_outcome == BudgetOutcome::ProvenBudgetConflict)
            .count();
        let precheck = entries
            .iter()
            .filter(|e| e.budget_outcome == BudgetOutcome::PreCheckRejected)
            .count();
        out.push_str(&format!(
            "- 发射预算裁决分布（G1 重新验收口径；采样候选耗尽 = 等 t 与等切向角划分均失败；\
             预算预检拒绝 = 0.105 m 采样质量目标的候选搜索前预检，非硬预算无解证明（R9 审计）；\
             已证预算冲突 = 真必要条件证明，当前发射层无此发射点）：采样候选耗尽 {exhausted}、\
             预算预检拒绝 {precheck}、已证预算冲突 {proven}、非预算类 {}。
",
            entries.len() - exhausted - precheck - proven
        ));
        out.push_str(&format!(
            "- 内车道不可行遍布 **{junctions} 个 junction**（XML 有内车道的 junction 共 1,942 个）。
"
        ));
        out.push_str(&format!(
            "- 内车道失败机制分布（两端均倒圆 → 硬折角倒圆；任一端边界钳制 → 边界钳制相关；\
             否则 → 纯内部 CR；authored 边不计入）：硬折角倒圆 {hard}、边界钳制相关 {clamp}、\
             纯内部 CR {cr}。

"
        ));
        out.push_str("## 全量清单（逐条诊断）\n\n");
        out.push_str("| # | lane | 类别 | junction | 机制 | 预算裁决 | span | chord(m) | 单片/均转(°) | 坐标 | start 切向 | finish 切向 |\n");
        out.push_str(
            "| ---: | --- | --- | --- | --- | --- | --- | ---: | ---: | --- | --- | --- |\n",
        );
        for (index, e) in entries.iter().enumerate() {
            let i = index + 1;
            let (span, chord, turn, coord, start, finish) = parse_diagnosis_fields(&e.entry);
            let cls = if e.is_internal {
                "SUMO 内车道"
            } else {
                "authored 边"
            };
            let junction =
                e.junction
                    .as_deref()
                    .unwrap_or(if e.is_internal { "(节点簇)" } else { "-" });
            out.push_str(&format!(
                "| {i} | `{}` | {cls} | {junction} | {} | {} | {span} | {chord} | {turn} | ({coord}) | {start} | {finish} |\n",
                e.lane_id,
                e.mechanism.label(),
                e.budget_outcome.label(),
            ));
        }
        let welded = weld_records
            .iter()
            .filter(|record| {
                record.disposition == crate::convert::junction::StubWeldDisposition::Welded
            })
            .count();
        out.push_str("\n## 点状 stub 删焊处置（已归一化 / 拒绝保留）\n\n");
        out.push_str(&format!(
            "规则版本 `{}`（G1 issuecomment-5901846033 六条件）；阈值（实际常量）：\
             位移 ≤ {} m、形状局部性 ≤ {} m、共享 join 间隙 ≤ {} m。焊接为全局入口端点改写，\
             共享入口关联穿越已逐条重验；拒绝保留类不删 stub、原始连接走正常穿越。\
             共 {} 条（已归一化 {} / 拒绝保留 {}）。\n\n",
            crate::convert::junction::STUB_WELD_RULE_VERSION,
            crate::convert::junction::STUB_WELD_MAX_DISPLACEMENT_M,
            crate::convert::junction::STUB_WELD_MAX_LOCALITY_M,
            crate::convert::junction::STUB_WELD_JOIN_GAP_M,
            weld_records.len(),
            welded,
            weld_records.len() - welded
        ));
        out.push_str("| stub lane | 入口 | 出口 | junction | 位移(m) | span(m) | shape长(m) | 局部性(m) | 受控 | 共享穿越 | 处置 |\n");
        out.push_str("| --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- | ---: | --- |\n");
        for record in weld_records {
            out.push_str(&format!(
                "| `{}` | `{}` | `{}` | `{}` | {:.4} | {:.4} | {:.4} | {:.4} | {} | {} | {} |\n",
                record.stub_lane_id,
                record.entry_lane_id,
                record.exit_lane_id,
                record.junction_id,
                record.displacement_m,
                record.span_m,
                record.shape_len_m,
                record.locality_m,
                if record.controlled { "是" } else { "否" },
                record.shared_traversal_count,
                record.disposition.label(),
            ));
        }
        out.push_str(
            "\n未支持-未评估：结构性 unsupported（如 stub 门控未过、控制语义检查）\
                      fail-closed 于转换之前，不产生清单条目。\n",
        );
        Self {
            entries,
            rendered: out,
        }
    }
}

/// 零长内部 lane 的合成微段长度（米）：取采样弦长下限（> HIR 冻结的
/// 0.1 m 退化段下限，`SPATIAL_MIN_SEGMENT_LENGTH_METERS`），方向取边界钳制，
/// 方向连续性由构造保证。
const DEGENERATE_SEGMENT_METERS: f64 = SAMPLE_MIN_CHORD_METERS;

/// 软/硬折点分界（度）：≤ 30° 视为采样伪影走平滑；> 30° 视为真实硬角走倒圆。
const SOFT_KINK_MAX_DEG: f64 = 30.0;
/// 倒圆半径上限（米，G1 修订冻结值）。
const FILLET_RADIUS_MAX_METERS: f64 = 5.0;
/// 倒圆单点最大偏离预算（米，G1 修订 issuecomment-5853823447：「偏离原折点
/// ≤ 5.0 m」）。半径选值受其约束 r ≤ 5/(sec(θ/2)−1)，构造出的 cubic 另做
/// 数值校验（#253 R7）。
///
/// 口径：预算在**参考 cubic** 上以本常量强制执行（终检取 cubic 到原折点的
/// 最近距离）。最终量化折线因 f32 端点量化可在参考值上叠加 ≤ 约 0.5 mm 的
/// 量化噪声（120° 参考 cubic 实测 gap 5.000 m → 量化折线 5.000255 m，
/// |坐标| ~7300 时 ulp ≈ 0.49 mm）——这是验收算术的固有粒度，不视为预算
/// 违反；若未来要求预算约束离散终态，需在选值时预留量化余量。
const FILLET_DEVIATION_MAX_METERS: f64 = 5.0;
/// 偏离校验数值扫描的均匀采样数（构造期一次，确定性）。
const FILLET_DEVIATION_SAMPLES: usize = 256;
/// 单侧回切占相邻弦长比例上限，防止相邻倒圆互相吞没中间弦。
const FILLET_CUTBACK_FRACTION: f64 = 0.45;
/// 直线判定阈值（弧度）：两端切向与弦的方向差均小于此值时直接发单条 `Line`。
const STRAIGHT_EPS_RAD: f64 = 1e-6;
/// 钳制方向与弦方向夹角超过 90° 时放弃钳制（回退自然弦向），避免 Bezier 打圈。
const CLAMP_GUARD_RAD: f64 = std::f64::consts::FRAC_PI_2;

type Vec3 = [f64; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// 单位化；近零向量返回 `None`。
fn unit(a: Vec3) -> Option<Vec3> {
    let len = norm(a);
    (len > 1e-12).then(|| scale(a, 1.0 / len))
}

/// 两个单位向量的夹角（弧度）。
fn angle_rad(u: Vec3, v: Vec3) -> f64 {
    dot(u, v).clamp(-1.0, 1.0).acos()
}

/// 角均值：单位向量和的方向。近对径（和近零）时回退到 `fallback`。
fn mean_dir(a: Vec3, b: Vec3, fallback: Vec3) -> Vec3 {
    unit(add(a, b)).unwrap_or(fallback)
}

fn point3(value: Vec3) -> Result<re::RoadEditingPoint3> {
    Ok(re::RoadEditingPoint3::try_new(
        value[0], value[1], value[2],
    )?)
}

// ---------------------------------------------------------------------------
// 边界切向钳制（策略 2）
// ---------------------------------------------------------------------------

/// maneuver 边界端点切向钳制表，按 LaneEdge id 索引。
pub(crate) struct BoundaryClamps {
    /// 首点切向钳制（单位方向）。
    start: HashMap<String, Vec3>,
    /// 末点切向钳制（单位方向）。
    finish: HashMap<String, Vec3>,
    /// 首点钳制的产生机制（fail-closed 诊断用）。
    start_source: HashMap<String, ClampSource>,
    /// 末点钳制的产生机制（fail-closed 诊断用）。
    finish_source: HashMap<String, ClampSource>,
}

/// 边界钳制的产生机制。fail-closed 报错时说明该端点切向由哪种机制产生，
/// 以区分 converter 引入的弯（钳制均值/跟随）与源数据自身的 authored 折角。
#[derive(Clone, Debug)]
pub(crate) enum ClampSource {
    /// 相邻对的候选目标（两侧端弦的角均值）。
    PairTarget,
    /// 扇出端的全部候选角均值；记录参与的 pair（"a->b" 边序）。
    FanOutMean(Vec<String>),
    /// 独占端跟随对端钳制；记录对端 edge id。
    PartnerFollow(String),
}

impl std::fmt::Display for ClampSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClampSource::PairTarget => {
                write!(f, "pair target (angular mean of both end chords)")
            }
            ClampSource::FanOutMean(pairs) => write!(
                f,
                "fan-out mean over maneuver boundary pairs [{}]",
                pairs.join(", ")
            ),
            ClampSource::PartnerFollow(partner) => {
                write!(f, "follows the clamp of partner edge {partner:?}")
            }
        }
    }
}

impl BoundaryClamps {
    pub(crate) fn at_start(&self, edge_id: &str) -> Option<Vec3> {
        self.start.get(edge_id).copied()
    }

    pub(crate) fn at_finish(&self, edge_id: &str) -> Option<Vec3> {
        self.finish.get(edge_id).copied()
    }

    pub(crate) fn source_at_start(&self, edge_id: &str) -> Option<&ClampSource> {
        self.start_source.get(edge_id)
    }

    pub(crate) fn source_at_finish(&self, edge_id: &str) -> Option<&ClampSource> {
        self.finish_source.get(edge_id)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Start,
    Finish,
}

/// 折线首/末端的非零弦方向（跳过重复点）；全部退化时返回 `None`。
fn end_chord_dir(points: &[Vec3], side: Side) -> Option<Vec3> {
    match side {
        Side::Start => points.windows(2).find_map(|w| unit(sub(w[1], w[0]))),
        Side::Finish => points.windows(2).rev().find_map(|w| unit(sub(w[1], w[0]))),
    }
}

/// 从 maneuver 路径的完整边序列收集边界钳制。
///
/// 对每个有序相邻对 (a, b)：候选目标 = a 末弦与 b 首弦的角均值。随后：
/// - 扇出端（出现在多个对中，如被多条路径共享的入口边末端）：钳制 = 全部候选的角均值；
/// - 独占端（只出现在一个对中）：跟随对端钳制值（对端扇出时）或候选目标（两端均独占时），
///   保证每一对的两侧钳制严格相等，边界跳变 ≈ 0。
pub(crate) fn boundary_clamps(
    traffic: &TrafficPackage,
    curve_by_edge: &HashMap<&str, &SpatialEdge>,
) -> Result<BoundaryClamps> {
    // 1. 去重后的有序相邻对。
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for path in &traffic.maneuver_paths {
        let mut chain: Vec<&str> = Vec::with_capacity(path.internal_edge_ids.len() + 2);
        chain.push(path.entry_edge_id.as_str());
        chain.extend(path.internal_edge_ids.iter().map(String::as_str));
        chain.push(path.exit_edge_id.as_str());
        for window in chain.windows(2) {
            if seen.insert((window[0], window[1])) {
                pairs.push((window[0], window[1]));
            }
        }
    }

    // 2. 每个对的候选目标方向。
    let chord = |edge_id: &str, side: Side| -> Result<Option<Vec3>> {
        let spatial = curve_by_edge.get(edge_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "maneuver path edge {edge_id:?} has no spatial centerline"
            ))
        })?;
        Ok(end_chord_dir(&spatial.centerline.points, side))
    };
    let mut targets: Vec<Option<Vec3>> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        let ua = chord(a, Side::Finish)?;
        let ub = chord(b, Side::Start)?;
        // 一侧零长（如 LuST 的 15 条点状内部 lane）时没有自有弦向，
        // 取非零侧弦向为目标——零长边的合成微段将沿该方向发出，边界自然对齐。
        targets.push(match (ua, ub) {
            (Some(ua), Some(ub)) => Some(mean_dir(ua, ub, ub)),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        });
    }

    // 3. 端点 → 对索引。
    let mut ends: HashMap<(String, Side), Vec<usize>> = HashMap::new();
    for (index, &(a, b)) in pairs.iter().enumerate() {
        ends.entry((a.to_owned(), Side::Finish))
            .or_default()
            .push(index);
        ends.entry((b.to_owned(), Side::Start))
            .or_default()
            .push(index);
    }

    // 4. 扇出端先定：全部候选的角均值。
    let mut clamp_of: HashMap<(String, Side), Vec3> = HashMap::new();
    let mut source_of: HashMap<(String, Side), ClampSource> = HashMap::new();
    for (key, pair_indices) in &ends {
        if pair_indices.len() > 1 {
            let dirs: Vec<Vec3> = pair_indices
                .iter()
                .filter_map(|&index| targets[index])
                .collect();
            if let Some(first) = dirs.first().copied() {
                let sum = dirs.iter().skip(1).fold(first, |acc, d| add(acc, *d));
                clamp_of.insert(key.clone(), unit(sum).unwrap_or(first));
                source_of.insert(
                    key.clone(),
                    ClampSource::FanOutMean(
                        pair_indices
                            .iter()
                            .map(|&index| {
                                let (a, b) = pairs[index];
                                format!("{a}->{b}")
                            })
                            .collect(),
                    ),
                );
            }
        }
    }

    // 5. 独占端跟随：对端扇出则取对端钳制（严格相等），两端均独占则取候选目标。
    for (key, pair_indices) in &ends {
        if pair_indices.len() != 1 || clamp_of.contains_key(key) {
            continue;
        }
        let index = pair_indices[0];
        let (a, b) = pairs[index];
        let partner = match key.1 {
            Side::Start => (a.to_owned(), Side::Finish),
            Side::Finish => (b.to_owned(), Side::Start),
        };
        let from_partner = clamp_of.get(&partner).copied();
        let value = from_partner.or(targets[index]);
        if let Some(dir) = value {
            clamp_of.insert(key.clone(), dir);
            // 跟随对端时继承对端已解析的产生机制（对端值已定，其机制描述同样
            // 适用于本端），诊断信息直接指向根因（如共享边界的扇出均值）。
            source_of.insert(
                key.clone(),
                match from_partner {
                    Some(_) => source_of
                        .get(&partner)
                        .cloned()
                        .unwrap_or(ClampSource::PartnerFollow(partner.0)),
                    None => ClampSource::PairTarget,
                },
            );
        }
    }

    let mut clamps = BoundaryClamps {
        start: HashMap::new(),
        finish: HashMap::new(),
        start_source: HashMap::new(),
        finish_source: HashMap::new(),
    };
    for ((edge_id, side), dir) in clamp_of {
        let source = source_of.remove(&(edge_id.clone(), side));
        match side {
            Side::Start => {
                clamps.start.insert(edge_id.clone(), dir);
                if let Some(source) = source {
                    clamps.start_source.insert(edge_id, source);
                }
            }
            Side::Finish => {
                clamps.finish.insert(edge_id.clone(), dir);
                if let Some(source) = source {
                    clamps.finish_source.insert(edge_id, source);
                }
            }
        }
    }
    Ok(clamps)
}

// ---------------------------------------------------------------------------
// 单条中心线的曲线修复（策略 1 + 3）
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Node {
    point: Vec3,
    /// 预设切向（单位方向）：倒圆切点专用；普通折点为 `None`（取 CR 方向）。
    prescribed: Option<Vec3>,
}

struct Span {
    from: Node,
    to: Node,
    /// 倒圆弧段的控制柄长度；非弧段为 `None`。
    arc_handle: Option<f64>,
}

/// 修复一条中心线并发射为 Road Editing 曲线程序。
///
/// `clamp_start` / `clamp_finish` 为首/末点的边界切向钳制（单位方向），
/// `clamp_start_source` / `clamp_finish_source` 为对应的产生机制（仅 fail-closed
/// 诊断用，不影响几何）。原始端点位置一律保持不变。
pub(crate) fn repair_curve(
    points: &[Vec3],
    clamp_start: Option<Vec3>,
    clamp_finish: Option<Vec3>,
    clamp_start_source: Option<&ClampSource>,
    clamp_finish_source: Option<&ClampSource>,
) -> Result<re::RoadEditingCurveProgram> {
    // 去重相邻重复点（SUMO shape 偶发）。
    let mut pts: Vec<Vec3> = Vec::with_capacity(points.len());
    for &p in points {
        if pts.last().is_none_or(|&last| norm(sub(p, last)) > 1e-9) {
            pts.push(p);
        }
    }
    let Some((&first, rest)) = pts.split_first() else {
        return Err(Error::SumoModel(
            "spatial edge centerline must hold at least one point".to_owned(),
        ));
    };
    if rest.is_empty() {
        // 真实 LuST 有 15 条零长内部 lane（shape 两点相同）。compiler 不接受空
        // segments，沿 maneuver 边界的钳制方向合成一段微段；不在任何路径中的
        // 零长边没有钳制来源，属于病态数据，fail-closed。
        let dir = match (clamp_start, clamp_finish) {
            (Some(start), Some(finish)) => mean_dir(start, finish, finish),
            (Some(only), None) | (None, Some(only)) => only,
            (None, None) => {
                return Err(Error::SumoModel(
                    "degenerate single-point centerline is not part of any maneuver path"
                        .to_owned(),
                ));
            }
        };
        let end = add(first, scale(dir, DEGENERATE_SEGMENT_METERS));
        return Ok(re::RoadEditingCurveProgram::try_new(
            point3(first)?,
            vec![re::RoadEditingCurveSegment::line(point3(end)?)],
        )?);
    }

    // 弦与折点角。
    let n = pts.len();
    let chords: Vec<Vec3> = pts.windows(2).map(|w| sub(w[1], w[0])).collect();
    let chord_lens: Vec<f64> = chords.iter().map(|c| norm(*c)).collect();
    let chord_units: Vec<Vec3> = chords
        .iter()
        .zip(&chord_lens)
        .map(|(c, l)| scale(*c, 1.0 / l))
        .collect();

    // 构建段序列：硬折点倒圆，其余保留。
    let soft_max_rad = SOFT_KINK_MAX_DEG.to_radians();
    let mut spans: Vec<Span> = Vec::with_capacity(n);
    let mut prev = Node {
        point: first,
        prescribed: None,
    };
    for i in 1..n - 1 {
        let v = pts[i];
        let u = chord_units[i - 1];
        let w = chord_units[i];
        let theta = angle_rad(u, w);
        if theta > soft_max_rad {
            let d_limit = FILLET_CUTBACK_FRACTION * chord_lens[i - 1].min(chord_lens[i]);
            let radius = fillet_radius(theta, d_limit);
            if radius > 1e-6 {
                let mut radius = radius;
                let mut d = radius * (theta * 0.5).tan();
                let mut t1 = add(v, scale(u, -d));
                let mut t2 = add(v, scale(w, d));
                // 圆弧的三次 Bezier 拟合：控制柄 (4/3)·r·tan(θ/4)，端点严格相切。
                let mut handle = (4.0 / 3.0) * radius * (theta * 0.25).tan();
                // 偏离预算终检（#253 R7）：cubic 弧到原折点的最近距离（切削
                // 深度，理想值 r·(sec(θ/2)−1)）。注意不是 max|p−v|——那量到的
                // 是回切端点距离 r·tan(θ/2)（恒更大），等于复现 tan 帽误拒。
                let mut gap = fillet_corner_gap(t1, t2, handle, u, w, v);
                if gap > FILLET_DEVIATION_MAX_METERS + 1e-9 {
                    // 恰界（θ=120°、r=5.0 时理想切削深度恰 5.000 m）：cubic 拟合
                    // 偏差可使实测略超预算。固定 θ 下深度 ≈ r·(sec(θ/2)−1) 对 r
                    // 线性，按预算比例回缩一次再终检（确定性）；仍超才 fail-closed。
                    radius *= FILLET_DEVIATION_MAX_METERS / gap;
                    d = radius * (theta * 0.5).tan();
                    t1 = add(v, scale(u, -d));
                    t2 = add(v, scale(w, d));
                    handle = (4.0 / 3.0) * radius * (theta * 0.25).tan();
                    gap = fillet_corner_gap(t1, t2, handle, u, w, v);
                }
                if gap > FILLET_DEVIATION_MAX_METERS + 1e-9 {
                    return Err(Error::SumoModel(format!(
                        "fillet arc deviates {gap:.3} m from the original corner at \
                         ({}, {}, {}), exceeding the {FILLET_DEVIATION_MAX_METERS} m budget",
                        v[0], v[1], v[2]
                    )));
                }
                let n1 = Node {
                    point: t1,
                    prescribed: Some(u),
                };
                let n2 = Node {
                    point: t2,
                    prescribed: Some(w),
                };
                spans.push(Span {
                    from: prev,
                    to: n1.clone(),
                    arc_handle: None,
                });
                spans.push(Span {
                    from: n1,
                    to: n2.clone(),
                    arc_handle: Some(handle),
                });
                prev = n2;
                continue;
            }
            // 弦长过短无法倒圆：保留原折点，由 compiler 对该病态数据亮诊断。
        }
        spans.push(Span {
            from: prev,
            to: Node {
                point: v,
                prescribed: None,
            },
            arc_handle: None,
        });
        prev = spans.last().expect("just pushed").to.clone();
    }
    spans.push(Span {
        from: prev,
        to: Node {
            point: pts[n - 1],
            prescribed: None,
        },
        arc_handle: None,
    });

    // 逐段发射：所有曲线统一采样为致密折线（Line 链）。
    //
    // 为什么不发 Bezier：compiler 对 Bezier 的 candidate 验收含"端点切向 vs 弦"
    // 半角检查（Balanced2Deg 半角 1°，对齐参考 Smooth1Deg 半角 0.5°），细分端点
    // 经 f32 量化——在 |坐标| ~7000 的 LuST 全网上 ulp ≈ 0.49mm，半径 < ~10m 的
    // 弯道在"几何对齐所需深度"处弦长已短到量化噪声超过半角预算，验收窗口不存在，
    // 必然 ApproximationNotConverged（实测 r≈4.3m 弯道在 5cm/2° 档 depth 6-8 处
    // 量化噪声 0.6-2.5° 淹没 1° 预算）。Line 段恒在 depth 0 验收，段间只做全角
    // weld 检查（2°），致密采样（切向转角 ≤ 1.2°/弦 + 矢高 ≤ 1cm + 弦长下限
    // 0.105m）并在发射时按量化后的真实弦做 weld 预检，可稳定通过。
    let mut sink = EmissionSink::new(first);
    for (j, span) in spans.iter().enumerate() {
        emit_sampled_span(j, span, clamp_start, clamp_finish, &spans, &mut sink).map_err(
            |error| {
                // fail-closed 诊断：报出失败跨段的位置与两端切向的产生机制，
                // 以区分 converter 引入的弯（边界钳制均值/跟随）与源数据自身的
                // authored 折角；几何裁决数字（弦长/转角/片数）在内层错误里。
                let chord = norm(sub(span.to.point, span.from.point));
                Error::SumoModel(format!(
                    "span {}/{} (chord {chord:.4} m) is not emittable: {}; \
                     start tangent: {}; finish tangent: {}",
                    j + 1,
                    spans.len(),
                    error,
                    tangent_source_desc(
                        &spans,
                        j,
                        Side::Start,
                        clamp_start,
                        clamp_start_source,
                    ),
                    tangent_source_desc(
                        &spans,
                        j,
                        Side::Finish,
                        clamp_finish,
                        clamp_finish_source,
                    ),
                ))
            },
        )?;
    }
    Ok(re::RoadEditingCurveProgram::try_new(
        point3(first)?,
        sink.segments,
    )?)
}

/// 采样发射的最大切向转角（弧度/弦）：1.2°。
///
/// 预算推导（全部针对 f32 量化后的规范弦，与 compiler 的验收算术一致）：
/// 段间 weld 全角预算 2°（Balanced2Deg 全角，`validate_canonical_polyline` 与
/// maneuver 边界 `check_spatial_direction` 同为该值）；量化噪声最坏
/// ≈ 0.7mm/弦长（|坐标| ~7300 时 ulp ≈ 0.49mm，两端点反向取整），
/// 在 0.105m 下限弦上约 0.67°。1.2° + 0.67° = 1.87° < 2°，留 0.13° 余量。
const SAMPLE_MAX_TURN_RAD: f64 = 1.2_f64 * std::f64::consts::PI / 180.0;
/// 采样矢高上限（米）：保证发射折线贴合修复曲线（远小于 5cm 精度档）。
const SAMPLE_MAX_SAGITTA_METERS: f64 = 0.01;
/// 采样弦长下限（米）：0.105。HIR 冻结拒绝长度 ≤ 0.1 m 的规范段
/// （`DegenerateSegment`，严格小于等于），f32 量化对弦长的扰动 ≤ 0.7mm，
/// 0.105 留出 25 倍余量。细分到该弦长仍未满足转角/矢高标准时按原样验收
/// （继续细分只会让量化噪声吃掉方向预算），由发射时的量化 weld 校验兜底。
const SAMPLE_MIN_CHORD_METERS: f64 = 0.105;
/// 发射时量化 weld 校验的方向预算（弧度）：1.95°，compiler 全角 2° 之下仅留
/// 0.05° 余量（覆盖 acos 与 cos² 比较在最后一位上的判定差）。取值依据：
/// 倒圆半径上限 5.0m 在下限弦 0.105m 上的几何转角 1.202° + 最坏量化噪声
/// 0.67° = 1.87°，必须稳过；量化后仍超预算即该处几何在本套验收常数下不可行
/// （曲率半径低于可行阈值，最坏情形约 4.7m），fail-closed 报精确位置，而不是
/// 让 compiler 在深处报 ApproximationNotConverged / DirectionDiscontinuity。
const WELD_MAX_TURN_RAD: f64 = 1.95_f64 * std::f64::consts::PI / 180.0;
/// 弦长下限区的单片安全转角（弧度）：1.9°。细分到弦长下限仍不满足 1.2°
/// 质量标准时，单片转角 ≤ 1.9° 才允许发射（sink 量化终检预算 1.95°）；
/// 超过即判定该处几何在本套验收常数下不可行，fail-closed。
const WELD_SAFE_TURN_RAD: f64 = 1.9_f64 * std::f64::consts::PI / 180.0;
/// 单段等分片数上限（防御病态输入）。
const SAMPLE_MAX_PIECES: u32 = 2048;
/// 等 t 初始候选失败后的等切向角回溯轮数（R6 复审；0 = 禁用回溯的归因
/// 对照，正常路径为 1：一轮等角重划分，与初始候选合计 ≤2 轮有界回溯）。
const SAMPLE_BACKTRACK_ROUNDS: u32 = 1;
/// HIR 冻结的退化段下限（米）：`SPATIAL_MIN_SEGMENT_LENGTH_METERS`，
/// 长度 ≤ 该值的规范段被拒绝。发射时按同一算术（f32 差分 + hypot）预检。
const QUANTIZED_MIN_SEGMENT_METERS: f32 = 0.1;

/// 倒圆半径选值（#253 R7）：半径上限、回切上限、偏离预算三者取最小。
///
/// 偏离预算的度量是弧矢高 r·(sec(θ/2)−1) ≤ 5.0 m（G1「偏离原折点
/// ≤ 5.0 m」），**不是**切点距离 r·tan(θ/2)——后者在 θ<180° 恒大于前者，
/// 用作约束会把 θ∈(约 94°, 134°) 的合规弯误压到 weld 容量可行半径下限
/// （≈3.19 m）之下（复审修正）。sec 界随 θ→180° 自然趋 0，无解边界在
/// 5/(sec(θ/2)−1) 低于可行下限处（约 θ ≳ 134°）。
fn fillet_radius(theta_rad: f64, cutback_limit: f64) -> f64 {
    let half = theta_rad * 0.5;
    let tan_half = half.tan();
    // 5/(sec(θ/2)−1) = 5·cos(θ/2)/(1−cos(θ/2))；用 sin² 半角恒等式
    // （1−cos x = 2 sin²(x/2)）避免小角相消。
    let cos_half = half.cos();
    let sagitta_cap =
        FILLET_DEVIATION_MAX_METERS * cos_half / (2.0 * (theta_rad * 0.25).sin().powi(2));
    (cutback_limit / tan_half)
        .min(FILLET_RADIUS_MAX_METERS)
        .min(sagitta_cap)
}

/// 倒圆 cubic（t1→t2，端点切向 u/w，控制柄 handle）到原折点 v 的最近距离
/// （切削深度）：均匀参数扫描 + 谷值附近一轮加密（确定性数值校验）。
///
/// 理想圆弧的闭合式为 r·(sec(θ/2)−1)，即 G1「偏离原折点 ≤ 5.0 m」的度量。
/// 必须取弧到折点的**最小**距离：取 max|p−v| 会量到回切端点距离
/// r·tan(θ/2)（θ<180° 恒更大），等于把回切距离当偏离预算，复现 tan 帽误拒。
fn fillet_corner_gap(t1: Vec3, t2: Vec3, handle: f64, u: Vec3, w: Vec3, v: Vec3) -> f64 {
    let c1 = add(t1, scale(u, handle));
    let c2 = sub(t2, scale(w, handle));
    let mut best_t = 0.0;
    let mut best = f64::INFINITY;
    for k in 0..=FILLET_DEVIATION_SAMPLES {
        let t = k as f64 / FILLET_DEVIATION_SAMPLES as f64;
        let d = norm(sub(cubic_at(t1, c1, c2, t2, t), v));
        if d < best {
            best = d;
            best_t = t;
        }
    }
    let refine = |k: usize, n: usize| (best_t + (k as f64 / n as f64 - 0.5) * 0.02).clamp(0.0, 1.0);
    let window = (0..=16).map(|k| refine(k, 16));
    for t in window {
        best = best.min(norm(sub(cubic_at(t1, c1, c2, t2, t), v)));
    }
    best
}

/// 发射水槽：持有已发射的 Line 链，并在每段入链前按 compiler 的验收算术
/// （端点 f32 量化 → 弦长 / 相邻弦全角）做确定性预检。
///
/// compiler 侧对应检查：
/// - HIR 冻结 `DegenerateSegment`：量化后弦长 ≤ 0.1 m 拒绝（spatial_freeze.rs）；
/// - road editing `DirectionDiscontinuity`：相邻量化弦全角 > 2°（Balanced2Deg）
///   拒绝（validate_canonical_polyline）。
///
/// 预检逐位复刻这两条（f32 差分 + hypot / f64 促升后夹角），失败即 fail-closed
/// 报量化坐标与实测值——该处几何在本套验收常数下不可行，诚实亮诊断。
struct EmissionSink {
    segments: Vec<re::RoadEditingCurveSegment>,
    /// 上一条已发射弦的量化终点（即程序量化首点或上一段终点）。
    prev_point: [f32; 3],
    /// 上一条已发射弦的量化方向（f64 促升）。
    prev_dir: Option<Vec3>,
}

/// 发射回溯检查点：记录已发射段数与链尾状态，O(1) 快照/回滚。
#[derive(Clone)]
struct SinkCheckpoint {
    segments_len: usize,
    prev_point: [f32; 3],
    prev_dir: Option<Vec3>,
}

impl EmissionSink {
    fn new(start: Vec3) -> Self {
        Self {
            segments: Vec::new(),
            prev_point: quantize_vec3(start),
            prev_dir: None,
        }
    }

    /// O(1) 快照：只记录段数与链尾，回滚靠 truncate（划分回溯轮用）。
    fn checkpoint(&self) -> SinkCheckpoint {
        SinkCheckpoint {
            segments_len: self.segments.len(),
            prev_point: self.prev_point,
            prev_dir: self.prev_dir,
        }
    }

    fn restore(&mut self, checkpoint: &SinkCheckpoint) {
        self.segments.truncate(checkpoint.segments_len);
        self.prev_point = checkpoint.prev_point;
        self.prev_dir = checkpoint.prev_dir;
    }

    fn push(&mut self, end: Vec3) -> Result<()> {
        let quantized = quantize_vec3(end);
        let delta = [
            f64::from(quantized[0]) - f64::from(self.prev_point[0]),
            f64::from(quantized[1]) - f64::from(self.prev_point[1]),
            f64::from(quantized[2]) - f64::from(self.prev_point[2]),
        ];
        // 与 HIR 冻结同一算术：f32 差分 + hypot。
        let length_f32 = (quantized[0] - self.prev_point[0])
            .hypot(quantized[1] - self.prev_point[1])
            .hypot(quantized[2] - self.prev_point[2]);
        if length_f32 <= QUANTIZED_MIN_SEGMENT_METERS {
            return Err(Error::SumoModel(format!(
                "repaired curve emits a degenerate canonical segment: quantized length \
                 {length_f32} m <= {QUANTIZED_MIN_SEGMENT_METERS} m at ({}, {}, {})",
                quantized[0], quantized[1], quantized[2]
            )));
        }
        let direction = unit(delta).expect("nonzero chord after length check");
        if let Some(prev) = self.prev_dir {
            let turn = angle_rad(prev, direction);
            if turn > WELD_MAX_TURN_RAD {
                return Err(Error::SumoModel(format!(
                    "repaired curve exceeds the quantized weld budget: {:.2} deg > {:.2} deg \
                     at ({}, {}, {}); local curvature is infeasible under the 2 deg full-angle \
                     acceptance with f32 endpoint quantization",
                    turn.to_degrees(),
                    WELD_MAX_TURN_RAD.to_degrees(),
                    quantized[0],
                    quantized[1],
                    quantized[2]
                )));
            }
        }
        self.segments
            .push(re::RoadEditingCurveSegment::line(point3(end)?));
        self.prev_point = quantized;
        self.prev_dir = Some(direction);
        Ok(())
    }
}

/// 端点量化：与 compiler `quantize_point` 同一语义（`as f32` 收缩）。
fn quantize_vec3(p: Vec3) -> [f32; 3] {
    [p[0] as f32, p[1] as f32, p[2] as f32]
}

/// 将一个逻辑段（直线 / CR 平滑 / 倒圆弧 / 钳制边界）采样发射为 Line 链。
fn emit_sampled_span(
    j: usize,
    span: &Span,
    clamp_start: Option<Vec3>,
    clamp_finish: Option<Vec3>,
    spans: &[Span],
    sink: &mut EmissionSink,
) -> Result<()> {
    let a = span.from.point;
    let b = span.to.point;
    let cv = sub(b, a);
    let Some(cu) = unit(cv) else {
        // 防御：零长段不应出现（去重与倒圆守卫下）；链式语义下静默跳过会
        // 破坏段间位置连续性，fail-closed。
        return Err(Error::SumoModel(
            "repaired curve contains a zero-length span".to_owned(),
        ));
    };
    let len = norm(cv);
    let (dir_a, dir_b) = if span.arc_handle.is_some() {
        (
            span.from.prescribed.expect("arc start tangent"),
            span.to.prescribed.expect("arc end tangent"),
        )
    } else {
        (
            tangent_dir(spans, j, Side::Start, clamp_start, cu),
            tangent_dir(spans, j, Side::Finish, clamp_finish, cu),
        )
    };
    if angle_rad(dir_a, cu) < STRAIGHT_EPS_RAD && angle_rad(dir_b, cu) < STRAIGHT_EPS_RAD {
        return sink.push(b);
    }
    // 弧段使用倒圆构造时算好的圆弧控制柄 (4/3)·r·tan(θ/4)（端点严格相切、
    // 曲率≈1/r）；其余段用 L/3 的 CR 式控制柄。
    let handle = span.arc_handle.unwrap_or(len / 3.0);
    let c1 = add(a, scale(dir_a, handle));
    let c2 = sub(b, scale(dir_b, handle));
    sample_cubic(a, c1, c2, b, dir_a, dir_b, sink)
}

/// 段 `j` 起点（`Side::Start`）或终点（`Side::Finish`）的切向。
///
/// 优先级：边界钳制（过 90° 守卫）→ 倒圆预设切向 → 端点自然弦向 /
/// 内部折点 CR 方向（前后节点的中心差分）。
fn tangent_dir(spans: &[Span], j: usize, side: Side, clamp: Option<Vec3>, chord: Vec3) -> Vec3 {
    let node = match side {
        Side::Start => &spans[j].from,
        Side::Finish => &spans[j].to,
    };
    if let Some(dir) = node.prescribed {
        return dir;
    }
    let at_end = match side {
        Side::Start => j == 0,
        Side::Finish => j + 1 == spans.len(),
    };
    if at_end {
        if let Some(dir) = clamp
            && angle_rad(dir, chord) <= CLAMP_GUARD_RAD
        {
            return dir;
        }
        return chord;
    }
    // CR 中心差分：(P_next - P_prev) 方向。
    let (prev_point, next_point) = match side {
        Side::Start => (spans[j - 1].from.point, spans[j].to.point),
        Side::Finish => (spans[j].from.point, spans[j + 1].to.point),
    };
    unit(sub(next_point, prev_point)).unwrap_or(chord)
}

/// 段端点切向的产生机制描述（与 `tangent_dir` 同一优先级；仅诊断用）。
fn tangent_source_desc(
    spans: &[Span],
    j: usize,
    side: Side,
    clamp: Option<Vec3>,
    clamp_source: Option<&ClampSource>,
) -> String {
    let node = match side {
        Side::Start => &spans[j].from,
        Side::Finish => &spans[j].to,
    };
    if node.prescribed.is_some() {
        return "fillet arc tangent (prescribed)".to_owned();
    }
    let at_end = match side {
        Side::Start => j == 0,
        Side::Finish => j + 1 == spans.len(),
    };
    if at_end {
        let chord = unit(sub(spans[j].to.point, spans[j].from.point));
        let guard_ok = match (clamp, chord) {
            (Some(dir), Some(chord)) => angle_rad(dir, chord) <= CLAMP_GUARD_RAD,
            _ => false,
        };
        if guard_ok {
            return match clamp_source {
                Some(source) => format!("boundary clamp ({source})"),
                None => "boundary clamp".to_owned(),
            };
        }
        return "natural chord direction (no clamp accepted)".to_owned();
    }
    "interior Catmull-Rom tangent".to_owned()
}

/// 三次 Bezier 在 t 处的点（弧长估计用）。
fn cubic_at(a: Vec3, c1: Vec3, c2: Vec3, b: Vec3, t: f64) -> Vec3 {
    let mt = 1.0 - t;
    let (w0, w1, w2, w3) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    [
        w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0],
        w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1],
        w0 * a[2] + w1 * c1[2] + w2 * c2[2] + w3 * b[2],
    ]
}

/// 跨段容量的确定性弧长估计：16 段内接折线累加。内接折线必不超过曲线真
/// 弧长，方向保守——#253 R6：整段端点弦长会系统性低估弧长（90° 圆弧
/// chord/arc ≈ 0.90），把可行几何误判为不可行。
fn cubic_arc_len(a: Vec3, c1: Vec3, c2: Vec3, b: Vec3) -> f64 {
    const SEGMENTS: usize = 16;
    let mut len = 0.0;
    let mut prev = a;
    for k in 1..=SEGMENTS {
        let p = cubic_at(a, c1, c2, b, k as f64 / SEGMENTS as f64);
        len += norm(sub(p, prev));
        prev = p;
    }
    len
}

/// de Casteljau 细分采样三次 Bezier 为 Line 链。
///
/// 片数按局部几何一次选定（质量目标 × 弧长容量下界），初始候选为等 t 参数
/// 划分；局部失败时回溯一轮做等切向角重划分（见 emit_angle_pieces），两轮
/// 都找不到合格候选报「sampling exhausted」（与已证预算冲突文案区分）。
fn sample_cubic(
    a: Vec3,
    c1: Vec3,
    c2: Vec3,
    b: Vec3,
    dir_a: Vec3,
    dir_b: Vec3,
    sink: &mut EmissionSink,
) -> Result<()> {
    let chord = sub(b, a);
    let turn = angle_rad(dir_a, dir_b);
    // 矢高估计：控制点到弦的最大距离（控制多边形保守上界）。
    let chord_len = norm(chord);
    let sagitta = if chord_len > 1e-12 {
        let cu = scale(chord, 1.0 / chord_len);
        let height = |p: Vec3| {
            let ap = sub(p, a);
            let along = dot(ap, cu);
            let perp = sub(ap, scale(cu, along));
            norm(perp)
        };
        height(c1).max(height(c2))
    } else {
        0.0
    };
    if turn <= SAMPLE_MAX_TURN_RAD && sagitta <= SAMPLE_MAX_SAGITTA_METERS {
        return sink.push(b);
    }
    // 质量目标所需片数（矢高随片数平方收缩）；弦长下限允许的最大片数。
    let n_quality = (turn / SAMPLE_MAX_TURN_RAD)
        .ceil()
        .max((sagitta / SAMPLE_MAX_SAGITTA_METERS).sqrt().ceil());
    // 容量按弧长估计（而非整段端点弦长，见 cubic_arc_len）。
    let span_len = cubic_arc_len(a, c1, c2, b);
    let n_floor = (span_len / SAMPLE_MIN_CHORD_METERS).floor();
    if n_floor < 2.0 {
        // R9 审计（质量目标预检，非硬预算证明）：n_floor 按 0.105 m 采样质量
        // 目标取整，硬下限是 0.1 m（sink/HIR 退化段）——floor(arc/0.105) < 2
        // 不排除「两段各 ≥0.1 m 且 ≤1.9°」的合法划分（复审数值反例：
        // arc 0.2066 m 预检拒绝，两段量化最短弦 0.1033 m > 0.1、weld
        // 1.8495° < 1.95° 全过）。此处报 "curvature is infeasible" 仅宣告
        // 质量目标预检拒绝，清单分类为 PreCheckRejected。单片在安全转角内
        // 仍发射（sink 量化终检），否则 fail-closed 报精确位置。
        if turn <= WELD_SAFE_TURN_RAD {
            return sink.push(b);
        }
        return Err(Error::SumoModel(format!(
            "repaired curve curvature is infeasible under the quantized weld budget: \
             span arc {span_len:.4} m (chord {chord_len:.4} m) turns {:.2} deg > {:.2} deg \
             at ({}, {}, {}); splitting finer would breach the {SAMPLE_MIN_CHORD_METERS} m \
             chord floor",
            turn.to_degrees(),
            WELD_SAFE_TURN_RAD.to_degrees(),
            b[0],
            b[1],
            b[2]
        )));
    }
    let n = n_quality.min(n_floor);
    if n > f64::from(SAMPLE_MAX_PIECES) {
        return Err(Error::SumoModel(format!(
            "repaired curve requires pathological sampling density: {n:.0} pieces for a \
             {chord_len:.4} m chord at ({}, {}, {})",
            b[0], b[1], b[2]
        )));
    }
    if n < n_quality && turn / n > WELD_SAFE_TURN_RAD {
        // R9 审计（质量目标预检，非硬预算证明）：n 被 floor(arc/0.105) 压住，
        // 0.105 是采样质量目标而非硬下限 0.1——「n 片每片超 1.9°」不证明
        // 硬预算无解（0.1 m 硬下限允许更多片）。分类为 PreCheckRejected。
        return Err(Error::SumoModel(format!(
            "repaired curve curvature is infeasible under the quantized weld budget: \
             {n:.0} pieces (chord floor) each turn {:.2} deg > {:.2} deg at ({}, {}, {})",
            (turn / n).to_degrees(),
            WELD_SAFE_TURN_RAD.to_degrees(),
            b[0],
            b[1],
            b[2]
        )));
    }
    // 候选划分序列（R6 复审）：round 0 等 t 初始候选（R6 初修路径，既有
    // 行为字节级不变）；round 1..=SAMPLE_BACKTRACK_ROUNDS 等切向角重划分——
    // 等 t 划分与非均匀曲率错配会让个别片切向超 1.9°（r=3.26 m 90° 倒圆
    // 48 片局部 1.9349°），而等切向角 48 片每片恰 1.875° 全预算内。轮数取尽
    // 仍失败报「sampling exhausted」（与已证预算冲突文案区分）；0 禁用回溯
    // （均匀划分的归因对照）。完全确定：固定轮次、固定容差、无随机/时钟。
    let n = n as u32;
    let span = BezierSpan {
        a,
        c1,
        c2,
        b,
        dir_a,
        dir_b,
    };
    let checkpoint = sink.checkpoint();
    for round in 0..=SAMPLE_BACKTRACK_ROUNDS {
        let result = if round == 0 {
            emit_uniform_pieces(&span, n, sink)
        } else {
            emit_angle_pieces(&span, n, sink)
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                sink.restore(&checkpoint);
                if round == SAMPLE_BACKTRACK_ROUNDS {
                    return Err(Error::SumoModel(format!(
                        "repaired curve sampling exhausted: {n} pieces breach the quantized \
                         weld budget under uniform and tangent-angle partitions at \
                         ({}, {}, {}); last error: {error}",
                        b[0], b[1], b[2]
                    )));
                }
            }
        }
    }
    unreachable!("loop returns by round {}", SAMPLE_BACKTRACK_ROUNDS);
}

/// 一段待采样的三次 Bezier 及其两端切向（束参数，避开 too_many_arguments）。
struct BezierSpan {
    a: Vec3,
    c1: Vec3,
    c2: Vec3,
    b: Vec3,
    dir_a: Vec3,
    dir_b: Vec3,
}

/// 等 t 参数切 n 片（de Casteljau），逐片递归重估：非均匀曲率下该片可
/// 自适应加密。sample_cubic 的初始候选划分。不用递归二分：二分网格（弦长
/// 逐次减半）会跨过"弦长下限 0.105m 与 weld 全角预算夹出的可行窗口"——
/// 例如 r=4.5m 倒圆的可行弦长窗口 (0.1, ~0.14]m，二分序列 …0.397 → 0.199
/// → 0.099 恰好全部落在窗外。等 n 切分可命中任意窗口；非均匀曲率造成的
/// 单片超差由递归重估（每片重新走一遍 sample_cubic，含一轮等角回溯）兜底。
fn emit_uniform_pieces(span: &BezierSpan, n: u32, sink: &mut EmissionSink) -> Result<()> {
    let mut rem = (span.a, span.c1, span.c2, span.dir_a);
    for k in 1..n {
        let t = 1.0 / f64::from(n - k + 1);
        let lerp = |p: Vec3, q: Vec3| add(p, scale(sub(q, p), t));
        let p01 = lerp(rem.0, rem.1);
        let p12 = lerp(rem.1, rem.2);
        let p23 = lerp(rem.2, span.b);
        let p012 = lerp(p01, p12);
        let p123 = lerp(p12, p23);
        let p0123 = lerp(p012, p123);
        // 分裂点切向：de Casteljau 第二层的差分方向。
        let dir_split = unit(sub(p123, p012)).unwrap_or(rem.3);
        // 逐片递归重估：非均匀曲率下该片可自适应加密。
        sample_cubic(rem.0, p01, p012, p0123, rem.3, dir_split, sink)?;
        // 剩余右子曲线：de Casteljau (p0123, p123, p23, b)。
        rem = (p0123, p123, p23, dir_split);
    }
    sample_cubic(rem.0, rem.1, rem.2, span.b, rem.3, span.dir_b, sink)
}

/// 等切向角重划分回溯轮：对 cubic 切向做密集累计角扫描，
/// 按累计切向角等分选切分参数（确定性），de Casteljau 切 n 片后逐片走
/// 常规采样。片数与初始候选相同（受 n_floor 约束），只重排参数分布。
fn emit_angle_pieces(span: &BezierSpan, n: u32, sink: &mut EmissionSink) -> Result<()> {
    let params = tangent_angle_split_params(span, n).ok_or_else(|| {
        Error::SumoModel(
            "tangent-angle partition unavailable: near-zero turn or degenerate tangent".to_owned(),
        )
    })?;
    let mut rem = (span.a, span.c1, span.c2, span.dir_a);
    let mut prev_t = 0.0_f64;
    for &t in &params[1..params.len() - 1] {
        // 参数是绝对量（原 cubic 的 t）；剩余子曲线的局部切分比例需按
        // 剩余参数长度归一（等 t 路径的 t 本来就是局部比例，无需此步）。
        let dt = (t - prev_t) / (1.0 - prev_t);
        let lerp = |p: Vec3, q: Vec3| add(p, scale(sub(q, p), dt));
        let p01 = lerp(rem.0, rem.1);
        let p12 = lerp(rem.1, rem.2);
        let p23 = lerp(rem.2, span.b);
        let p012 = lerp(p01, p12);
        let p123 = lerp(p12, p23);
        let p0123 = lerp(p012, p123);
        let dir_split = unit(sub(p123, p012)).unwrap_or(rem.3);
        sample_cubic(rem.0, p01, p012, p0123, rem.3, dir_split, sink)?;
        rem = (p0123, p123, p23, dir_split);
        prev_t = t;
    }
    sample_cubic(rem.0, rem.1, rem.2, span.b, rem.3, span.dir_b, sink)
}

/// 等切向角切分参数：均匀参数扫描切向角并累计，按累计角等分（k/n 目标）
/// 线性插值出切分参数（固定 512 点扫描，确定性；无随机/时钟）。
fn tangent_angle_split_params(span: &BezierSpan, n: u32) -> Option<Vec<f64>> {
    const SCAN: usize = 512;
    let mut prev_dir = unit(cubic_tangent_at(span.a, span.c1, span.c2, span.b, 0.0))?;
    let mut cumulative = vec![0.0_f64; SCAN + 1];
    for k in 1..=SCAN {
        let dir = unit(cubic_tangent_at(
            span.a,
            span.c1,
            span.c2,
            span.b,
            k as f64 / SCAN as f64,
        ))
        .unwrap_or(prev_dir);
        cumulative[k] = cumulative[k - 1] + angle_rad(prev_dir, dir);
        prev_dir = dir;
    }
    let total = cumulative[SCAN];
    if total <= 1e-12 {
        return None;
    }
    let mut params = vec![0.0_f64; n as usize + 1];
    *params.last_mut().expect("n + 1 entries") = 1.0;
    let mut scan = 0_usize;
    for (k, slot) in params.iter_mut().enumerate().take(n as usize).skip(1) {
        let target = total * k as f64 / n as f64;
        while scan + 1 < cumulative.len() && cumulative[scan + 1] < target {
            scan += 1;
        }
        let t = if cumulative[scan + 1] > cumulative[scan] {
            let f = (target - cumulative[scan]) / (cumulative[scan + 1] - cumulative[scan]);
            (scan as f64 + f) / SCAN as f64
        } else {
            scan as f64 / SCAN as f64
        };
        *slot = t.clamp(0.0, 1.0);
    }
    Some(params)
}

/// 三次 Bezier 在 t 处的切向（导数方向，未归一）。
fn cubic_tangent_at(a: Vec3, c1: Vec3, c2: Vec3, b: Vec3, t: f64) -> Vec3 {
    let mt = 1.0 - t;
    let w0 = 3.0 * mt * mt;
    let w1 = 6.0 * mt * t;
    let w2 = 3.0 * t * t;
    [
        w0 * (c1[0] - a[0]) + w1 * (c2[0] - c1[0]) + w2 * (b[0] - c2[0]),
        w0 * (c1[1] - a[1]) + w1 * (c2[1] - c1[1]) + w2 * (b[1] - c2[1]),
        w0 * (c1[2] - a[2]) + w1 * (c2[2] - c1[2]) + w2 * (b[2] - c2[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Vec3 {
        [x, y, 0.0]
    }

    fn seg_ends(program: &re::RoadEditingCurveProgram) -> usize {
        program.segments().len()
    }

    #[test]
    fn straight_polyline_stays_line_segments() {
        let points = [pt(0.0, 0.0), pt(10.0, 0.0), pt(20.0, 0.0)];
        let program = repair_curve(&points, None, None, None, None).expect("repair");
        assert_eq!(seg_ends(&program), 2);
        for segment in program.segments() {
            assert!(matches!(
                segment.geometry(),
                re::RoadEditingCurveSegmentGeometry::Line { .. }
            ));
        }
    }

    #[test]
    fn soft_bend_becomes_sampled_polyline() {
        // 10°/弦的 90° 圆弧采样：全部软折点 → CR 参考曲线按 ≤1.2°/弦致密采样。
        let mut points = vec![pt(0.0, 0.0)];
        for k in 1..=9 {
            let a = (k as f64 * 10.0).to_radians();
            points.push(pt(10.0 * a.sin(), 10.0 * (1.0 - a.cos())));
        }
        let program = repair_curve(&points, None, None, None, None).expect("repair");
        // 90° 总转角按 1.2°/弦采样：段数应远多于原折点链（9），且全部为 Line。
        assert!(seg_ends(&program) > 40, "expected dense sampling");
        let mut prev: Vec3 = {
            let start = program.start();
            [start.x(), start.y(), start.z()]
        };
        let mut prev_dir: Option<Vec3> = None;
        for segment in program.segments() {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                panic!("sampled emission must be all-Line");
            };
            let b: Vec3 = [end.x(), end.y(), end.z()];
            let chord = sub(b, prev);
            assert!(
                norm(chord) > 0.1,
                "sampled chord must clear the HIR degenerate-segment minimum"
            );
            let dir = unit(chord).expect("nonzero chord");
            if let Some(before) = prev_dir {
                let jump = angle_rad(before, dir).to_degrees();
                assert!(jump < 1.5, "chord-to-chord jump {jump} deg too large");
            }
            prev_dir = Some(dir);
            prev = b;
        }
        // 端点保持原位。
        assert!((prev[0] - points[9][0]).abs() < 1e-9);
        assert!((prev[1] - points[9][1]).abs() < 1e-9);
    }

    #[test]
    fn hard_corner_gets_bounded_fillet() {
        // 90° 硬角，两侧弦足够长：r 取满 5.0 m，回切 d = r·tan(45°) = 5.0。
        let points = [pt(0.0, 0.0), pt(20.0, 0.0), pt(20.0, 20.0)];
        let program = repair_curve(&points, None, None, None, None).expect("repair");
        // 倒圆弧经采样后为多段 Line；弧段终点 T2 = V + w·d = (20, 5) 必须是
        // 某个采样弦的端点（采样在弧段边界处对齐）。
        let has_t2 = program.segments().iter().any(|segment| {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                return false;
            };
            (end.x() - 20.0).abs() < 1e-9 && (end.y() - FILLET_RADIUS_MAX_METERS).abs() < 1e-9
        });
        assert!(has_t2, "fillet arc must end at T2=(20, 5)");
        assert!(seg_ends(&program) > 3, "fillet arc must be densely sampled");
    }

    #[test]
    fn fillet_radius_bounded_by_short_chords() {
        // 90° 硬角，引出弦 10 m：d ≤ 0.45·10 = 4.5 < 5.0 上限 → r = 4.5，
        // 仍高于量化 weld 的可行半径阈值（最坏约 4.7m 以下的典型区），采样可通过。
        let points = [pt(0.0, 0.0), pt(20.0, 0.0), pt(20.0, 10.0)];
        let program = repair_curve(&points, None, None, None, None).expect("repair");
        let has_t2 = program.segments().iter().any(|segment| {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                return false;
            };
            (end.x() - 20.0).abs() < 1e-6 && (end.y() - 4.5).abs() < 1e-6
        });
        assert!(has_t2, "fillet arc must end at T2=(20, 4.5)");
    }

    #[test]
    fn tight_fillet_fails_closed_with_precise_error() {
        // 90° 硬角但引出弦只有 4 m：d ≤ 1.8 → r = 1.8。该半径在 2° 全角 +
        // f32 量化 + 0.1m 退化段下限的验收常数下不可行（下限弦转角 3.3°），
        // 发射时的量化 weld 预检必须 fail-closed 并报出方向预算，而不是把
        // 病态几何交给 compiler 在深处冻结。
        let points = [pt(0.0, 0.0), pt(20.0, 0.0), pt(20.0, 4.0)];
        let err =
            repair_curve(&points, None, None, None, None).expect_err("tight fillet must fail");
        let message = err.to_string();
        assert!(
            message.contains("weld budget"),
            "error must name the weld budget, got: {message}"
        );
    }

    #[test]
    fn clamped_two_point_edge_bends_toward_clamp() {
        // 2 点边 + 末端钳制偏离弦向：采样折线，末弦方向趋近钳制方向；
        // 钳制与弦共线：单条 Line。
        let points = [pt(0.0, 0.0), pt(10.0, 0.0)];
        let tilt = unit([1.0, 0.1, 0.0]).expect("unit");
        let program = repair_curve(&points, None, Some(tilt), None, None).expect("repair");
        assert!(seg_ends(&program) >= 1);
        let re::RoadEditingCurveSegmentGeometry::Line { end } =
            program.segments().last().expect("last").geometry()
        else {
            panic!("sampled emission must be all-Line");
        };
        // 端点位置不变；末弦与钳制的夹角应显著小于原弦与钳制的夹角（5.7°）。
        assert!((end.x() - 10.0).abs() < 1e-9 && end.y().abs() < 1e-9);
        let program =
            repair_curve(&points, None, Some([1.0, 0.0, 0.0]), None, None).expect("repair");
        assert_eq!(seg_ends(&program), 1);
        assert!(matches!(
            program.segments()[0].geometry(),
            re::RoadEditingCurveSegmentGeometry::Line { .. }
        ));
    }

    #[test]
    fn clamp_guard_rejects_opposite_direction() {
        // 钳制与弦近对径（> 90°）：回退自然弦向，保持 Line。
        let points = [pt(0.0, 0.0), pt(10.0, 0.0)];
        let program =
            repair_curve(&points, None, Some([-1.0, 0.0, 0.0]), None, None).expect("repair");
        assert!(matches!(
            program.segments()[0].geometry(),
            re::RoadEditingCurveSegmentGeometry::Line { .. }
        ));
    }

    #[test]
    fn arc_capacity_uses_arc_length_not_chord() {
        // #253 R6 复审反例：r=3.5 m 90° 倒圆 cubic。整段端点弦长 4.9497 m →
        // 旧容量 47 片误拒（90/47 = 1.915° > 1.9°）；弧长 5.4978 m → 52 片，
        // 逐片 ≤1.8°，可行。两种弦长下限语义：0.105 m 是采样质量目标（尽力），
        // 个别片可落到其下；0.1 m 是 sink/HIR 退化段硬下限，必须全片满足
        // （Python 镜像：52 片最短量化弦 ≈ 0.103352 m，落在这两限之间）。
        let a = [16.5, 0.0, 0.0];
        let b = [20.0, 0.0, 3.5];
        let h = (4.0 / 3.0) * 3.5 * (std::f64::consts::FRAC_PI_8).tan();
        let c1 = add(a, scale([1.0, 0.0, 0.0], h));
        let c2 = sub(b, scale([0.0, 0.0, 1.0], h));
        let mut sink = EmissionSink::new(a);
        sample_cubic(a, c1, c2, b, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &mut sink)
            .expect("arc-length capacity must emit the R6 counterexample");
        // 从发射出的 Line 链复核：片数、逐片量化前几何转角与量化弦长。
        assert!(
            sink.segments.len() >= 50,
            "expected at least 50 pieces, got {}",
            sink.segments.len()
        );
        let mut prev = a;
        let mut prev_dir: Option<Vec3> = None;
        let mut max_turn = 0.0_f64;
        let mut min_chord = f64::INFINITY;
        for segment in &sink.segments {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                panic!("sampled emission must be all-Line");
            };
            let next = [end.x(), end.y(), end.z()];
            let chord = sub(next, prev);
            min_chord = min_chord.min(norm(chord));
            let dir = unit(chord).expect("nonzero chord");
            if let Some(before) = prev_dir {
                max_turn = max_turn.max(angle_rad(before, dir).to_degrees());
            }
            prev_dir = Some(dir);
            prev = next;
        }
        assert!(max_turn <= 1.95, "peak piece turn {max_turn}");
        assert!(
            min_chord > 0.1,
            "every quantized chord must clear the HIR 0.1 m degenerate floor, got {min_chord}"
        );
    }

    #[test]
    fn tangent_angle_partition_rescues_r326_counterexample() {
        // #253 R6 复审反例：r=3.26 m 90° 倒圆 cubic（a=(20−r,0,0)、b=(20,0,r)）。
        // 16 段弧长估计 5.1195 m → 48 片；等 t 48 片的局部片切向 1.9349° > 1.9°
        // 被误拒（递归落到 n_floor<2 单弦回退）；等切向角 48 片每片恰 1.875°，
        // 量化最短弦 0.10583 m > 0.105 m、最大 weld 角 1.8766° < 1.95°，全预算内。
        // 回溯轮必须放行，且逐片几何转角 ≤ 1.9°。
        let r = 3.26;
        let a = [20.0 - r, 0.0, 0.0];
        let b = [20.0, 0.0, r];
        let h = (4.0 / 3.0) * r * (std::f64::consts::FRAC_PI_8).tan();
        let c1 = add(a, scale([1.0, 0.0, 0.0], h));
        let c2 = sub(b, scale([0.0, 0.0, 1.0], h));
        let mut sink = EmissionSink::new(a);
        sample_cubic(a, c1, c2, b, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], &mut sink)
            .expect("tangent-angle backtrack must emit the r=3.26 counterexample");
        assert!(
            sink.segments.len() >= 48,
            "expected at least 48 pieces, got {}",
            sink.segments.len()
        );
        let mut prev = a;
        let mut prev_dir: Option<Vec3> = None;
        let mut max_turn = 0.0_f64;
        for segment in &sink.segments {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                panic!("sampled emission must be all-Line");
            };
            let next = [end.x(), end.y(), end.z()];
            let dir = unit(sub(next, prev)).expect("nonzero chord");
            if let Some(before) = prev_dir {
                max_turn = max_turn.max(angle_rad(before, dir).to_degrees());
            }
            prev_dir = Some(dir);
            prev = next;
        }
        assert!(
            max_turn <= 1.9,
            "tangent-angle pieces must stay within the 1.9 deg safe turn, got {max_turn}"
        );
    }

    #[test]
    fn fillet_radius_respects_deviation_budget() {
        // #253 R7：偏离预算约束 r ≤ 5/(sec(θ/2)−1)（弧矢高；G1「偏离原折点
        // ≤ 5.0 m」）。回切不绑定（big）时 90°/100°/120° 取满半径上限 5.0
        // （120° 恰界：sec60°−1 = 1，r = 5 的矢高恰 5.000 m），150° 压至
        // 5/(sec75°−1) = 1.746。约束不是切点距离 r·tan(θ/2)——后者恒更严，
        // 会把 θ∈(约 94°, 134°) 的合规弯误压到 weld 容量下限之下。
        let big = 1.0e9;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(near(fillet_radius(std::f64::consts::FRAC_PI_2, big), 5.0));
        assert!(near(fillet_radius(100.0_f64.to_radians(), big), 5.0));
        assert!(near(fillet_radius(120.0_f64.to_radians(), big), 5.0));
        assert!(near(fillet_radius(150.0_f64.to_radians(), big), 1.7460));
    }

    #[test]
    fn wide_corner_fillet_fails_closed_within_deviation_budget() {
        // #253 R7：150° 硬角在偏离预算下半径被压至 1.746 m，倒圆在量化 weld
        // 预算内不可行 → fail-closed（旧行为 r=5 可发射但偏离原折点 14.3 m，
        // 违反 G1「偏离原折点 ≤ 5.0 m」）。
        // 25 m 邻弦：回切上限 (0.45·25/tan75° ≈ 3.01) 高于偏离预算
        // (5/(sec75°−1) ≈ 1.746)，确保压半径的是偏离预算本身。
        let c150 = 150.0_f64.to_radians();
        let points = [
            pt(0.0, 0.0),
            pt(25.0, 0.0),
            pt(25.0 + 25.0 * c150.cos(), 25.0 * c150.sin()),
        ];
        let err = repair_curve(&points, None, None, None, None)
            .expect_err("150 deg fillet must fail closed");
        let message = err.to_string();
        assert!(
            message.contains("weld budget"),
            "expected weld-budget infeasibility, got: {message}"
        );
    }

    #[test]
    fn deviation_capped_fillet_still_emits() {
        // 100°（25 m 邻弦）：复审修正的正向锁定——半径取满 5.0（旧 tan 帽
        // 会误压至 4.196），切削深度 5·(sec50°−1) ≈ 2.779 m 远在预算内，倒圆
        // 发射。min-gap 锁定排除 tan 帽回归（tan 帽下 min-gap ≈ 2.332）。
        let c100 = 100.0_f64.to_radians();
        let points = [
            pt(0.0, 0.0),
            pt(25.0, 0.0),
            pt(25.0 + 25.0 * c100.cos(), 25.0 * c100.sin()),
        ];
        let program = repair_curve(&points, None, None, None, None)
            .expect("100 deg fillet emits within budget");
        // 倒圆几何（原 lane 两个端点除外）到原折点的最近距离：切削深度。
        let corner = [25.0, 0.0, 0.0];
        let start = program.start();
        let mut min_gap = f64::INFINITY;
        let mut prev: Vec3 = [start.x(), start.y(), start.z()];
        for (index, segment) in program.segments().iter().enumerate() {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                panic!("sampled emission must be all-Line");
            };
            let next: Vec3 = [end.x(), end.y(), end.z()];
            // 首段起点是原 lane 起点，不属于倒圆几何。
            if index > 0 {
                min_gap = min_gap.min(norm(sub(prev, corner)));
            }
            prev = next;
        }
        assert!(
            (2.75..=2.85).contains(&min_gap),
            "cut depth {min_gap} m must match r=5.0 (tan cap would give ~2.332)"
        );
        assert!(min_gap <= FILLET_DEVIATION_MAX_METERS + 1e-9);
    }

    #[test]
    fn boundary_corner_fillet_keeps_full_radius() {
        // 120° 恰界（25 m 邻弦）：半径取满 5.0，理想切削深度恰 5.000 m，
        // cubic 实测不越预算（不回缩），T2 = V + w·5·tan60° = (25−4.330127, 7.5)
        // 必须作为采样弦端点出现——锁定「恰界放行、半径不被回缩」。
        let c120 = 120.0_f64.to_radians();
        let points = [
            pt(0.0, 0.0),
            pt(25.0, 0.0),
            pt(25.0 + 25.0 * c120.cos(), 25.0 * c120.sin()),
        ];
        let program = repair_curve(&points, None, None, None, None)
            .expect("120 deg boundary fillet emits at full radius");
        let d = FILLET_RADIUS_MAX_METERS * (c120 * 0.5).tan();
        let has_t2 = program.segments().iter().any(|segment| {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                return false;
            };
            let expected_x = 25.0 + c120.cos() * d;
            let expected_y = c120.sin() * d;
            (end.x() - expected_x).abs() < 1e-6 && (end.y() - expected_y).abs() < 1e-6
        });
        assert!(
            has_t2,
            "fillet arc must end at T2 at full radius (no shrink)"
        );
    }

    #[test]
    fn fillet_corner_gap_matches_sagitta_model() {
        // 切削深度（cubic 到原折点最近距离）对理想弧闭合式 r·(sec(θ/2)−1) 的
        // 数值校验；同时锁 gap < 回切距离 r·tan(θ/2)（排除 max|p−v| 语义回归）。
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        let gap_at = |theta_deg: f64, r: f64| {
            let theta = theta_deg.to_radians();
            let d = r * (theta * 0.5).tan();
            let u = unit([1.0, 0.0, 0.0]).expect("unit");
            let w = [theta.cos(), 0.0, theta.sin()];
            let v = [0.0, 0.0, 0.0];
            let t1 = add(v, scale(u, -d));
            let t2 = add(v, scale(w, d));
            let handle = (4.0 / 3.0) * r * (theta * 0.25).tan();
            fillet_corner_gap(t1, t2, handle, u, w, v)
        };
        let ideal = |theta_deg: f64, r: f64| {
            let half = theta_deg.to_radians() * 0.5;
            r * (half.cos().recip() - 1.0)
        };
        for (theta, r) in [(90.0, 5.0), (120.0, 5.0), (150.0, 1.7460)] {
            let gap = gap_at(theta, r);
            assert!(
                near(gap, ideal(theta, r)),
                "gap {gap} != ideal {} at theta={theta} r={r}",
                ideal(theta, r)
            );
            assert!(
                gap < r * (theta.to_radians() * 0.5).tan(),
                "gap must be cut depth, not cutback distance"
            );
        }
    }

    #[test]
    fn deviation_budget_makes_wide_corners_infeasible_not_violating() {
        // 140°（25 m 邻弦）：偏离预算 r ≤ 5/(sec70°−1) = 2.599，低于 weld 容量
        // 可行半径下限（≈3.19 m），两预算无交——fail-closed 必须是发射层拒绝，
        // 而不是发出一个偏离原折点 >5 m 的倒圆。sec 界下无解边界在
        // 5/(sec(θ/2)−1) < 3.19 m 处，即 θ ≳ 134°。
        let c140 = 140.0_f64.to_radians();
        let points = [
            pt(0.0, 0.0),
            pt(25.0, 0.0),
            pt(25.0 + 25.0 * c140.cos(), 25.0 * c140.sin()),
        ];
        let err = repair_curve(&points, None, None, None, None)
            .expect_err("140 deg fillet is infeasible under the joint budgets");
        let message = err.to_string();
        assert!(
            message.contains("weld budget") || message.contains("deviates"),
            "unexpected failure mode: {message}"
        );
    }

    #[test]
    fn infeasible_span_error_names_tangent_sources() {
        // LuST #253 全网失败的等价缩影：尾弦短、边界钳制与 CR 切向夹角大，
        // 尾段扫掠角超出弦长下限片数 × 安全转角的容量。fail-closed 错误必须
        // 报出失败跨段的位置与两端切向的产生机制（区分 converter 引入的弯与
        // 源数据自身的 authored 折角）。折点保持软折（< 30°），让失败落在
        // 钳制尾段而非倒圆弧段。
        let points = [pt(0.0, 0.0), pt(2.0, 0.5), pt(2.448, 0.876)];
        let sweep_clamp = unit([-0.1736, 0.9848, 0.0]).expect("unit"); // 100°
        let source = ClampSource::FanOutMean(vec!["sibling->exit".to_owned()]);
        let err = repair_curve(&points, None, Some(sweep_clamp), None, Some(&source))
            .expect_err("overloaded trailing span must fail");
        let message = err.to_string();
        assert!(
            message.contains("span 2/2"),
            "error must name the failing span, got: {message}"
        );
        assert!(
            message.contains("weld budget"),
            "error must name the weld budget, got: {message}"
        );
        assert!(
            message.contains(
                "boundary clamp (fan-out mean over maneuver boundary pairs [sibling->exit])"
            ),
            "error must name the clamp provenance, got: {message}"
        );
        assert!(
            message.contains("interior Catmull-Rom tangent"),
            "error must name the start tangent source, got: {message}"
        );
    }

    #[test]
    fn duplicate_points_are_deduplicated() {
        let points = [pt(0.0, 0.0), pt(0.0, 0.0), pt(10.0, 0.0)];
        let program = repair_curve(&points, None, None, None, None).expect("repair");
        assert_eq!(seg_ends(&program), 1);
    }

    #[test]
    fn classify_distinguishes_budget_outcomes() {
        // G1 六条件之重新验收：清单逐条如实区分「采样候选耗尽」与「已证预算
        // 冲突」（R6 错误文案拆分），其余失败为非预算类。
        let exhausted = classify_infeasible(
            "sumo::-1000_2_0",
            "span 1/1 (chord 1.0 m) is not emittable: repaired curve sampling exhausted: 2 pieces breach the quantized weld budget under uniform and tangent-angle partitions at (1, 0, 2); last error: x; start tangent: interior Catmull-Rom tangent; finish tangent: interior Catmull-Rom tangent"
                .to_owned(),
            true,
        );
        assert_eq!(exhausted.budget_outcome, BudgetOutcome::SamplerExhausted);
        // R9："curvature is infeasible" 只有质量目标预检一个来源 → 预检拒绝，
        // 不再冒充已证预算冲突（ProvenBudgetConflict 审计后当前无发射点）。
        let precheck = classify_infeasible(
            "sumo::-1000_2_0",
            "span 1/1 (chord 1.0 m) is not emittable: repaired curve curvature is infeasible under the quantized weld budget: span arc 0.11 m turns 1.93 deg at (1, 0, 2); start tangent: interior Catmull-Rom tangent; finish tangent: interior Catmull-Rom tangent"
                .to_owned(),
            true,
        );
        assert_eq!(precheck.budget_outcome, BudgetOutcome::PreCheckRejected);
        assert_ne!(precheck.budget_outcome, BudgetOutcome::ProvenBudgetConflict);
        let other = classify_infeasible(
            "sumo::-1000_2_0",
            "span 1/1 (chord 1.0 m) is not emittable: fillet arc deviates 5.1 m from the original corner at (1, 0, 2), exceeding the 5.0 m budget; start tangent: interior Catmull-Rom tangent; finish tangent: interior Catmull-Rom tangent"
                .to_owned(),
            true,
        );
        assert_eq!(other.budget_outcome, BudgetOutcome::NotBudget);
    }

    #[test]
    fn quality_floor_precheck_is_not_proven_conflict() {
        // R9 复审数值反例：L/3 控制柄 cubic（起止切向夹角 3.7°，16 段弧长
        // 估计 0.206646828 m → n_floor = floor(0.2066/0.105) = 1 < 2）。预检
        // 拒绝它，但同曲线两段划分量化最短弦 0.103318997 m > 0.1 m、最大
        // weld 1.849517822° < 1.95°、子段切向 1.85° < 1.9°、垂距上界
        // 0.000556053 m < 0.01 m——满足全部硬预算。故诊断分类必须是
        // PreCheckRejected，不得标 ProvenBudgetConflict。
        let a = [0.0, 0.0, 0.0];
        let c1 = [0.06887035952165224, 0.0, 0.0];
        let c2 = [0.1377765790067887, 0.0, 0.002225658258883879];
        let b = [0.20650338640946556, 0.0, 0.006670021529027181];
        let dir_a = [1.0, 0.0, 0.0];
        let dir_b = unit([b[0] - c2[0], b[1] - c2[1], b[2] - c2[2]]).expect("unit");
        let mut sink = EmissionSink::new(a);
        let error = sample_cubic(a, c1, c2, b, dir_a, dir_b, &mut sink)
            .expect_err("quality-floor precheck rejects the counterexample");
        assert!(
            error.to_string().contains("curvature is infeasible"),
            "unexpected error: {error}"
        );
        // 走实际清单分类路径（span 包装 + 切向来源与发射层一致）。
        let wrapped = format!(
            "span 1/1 (chord 0.2065 m) is not emittable: {error}; start tangent: interior \
             Catmull-Rom tangent; finish tangent: interior Catmull-Rom tangent"
        );
        let diagnosis = classify_infeasible("sumo::counterexample_0", wrapped, true);
        assert_eq!(
            diagnosis.budget_outcome,
            BudgetOutcome::PreCheckRejected,
            "quality-target precheck must not be labelled proven conflict"
        );
        assert_ne!(
            diagnosis.budget_outcome,
            BudgetOutcome::ProvenBudgetConflict
        );
    }

    #[test]
    fn render_includes_weld_records_and_outcome_column() {
        // G1 可追溯 + 四类口径：清单渲染含预算裁决列、发射预算裁决分布、
        // stub 删焊处置记录附录与「未支持-未评估」说明。
        let entries = vec![classify_infeasible(
            "sumo::lane_0",
            "span 1/1 (chord 1.0 m) is not emittable: repaired curve sampling exhausted: 2 pieces breach the quantized weld budget; start tangent: interior Catmull-Rom tangent; finish tangent: interior Catmull-Rom tangent"
                .to_owned(),
            true,
        )];
        let records = vec![crate::convert::junction::StubWeldRecord {
            rule_version: crate::convert::junction::STUB_WELD_RULE_VERSION,
            disposition: crate::convert::junction::StubWeldDisposition::Welded,
            stub_lane_id: ":J_0_0".to_owned(),
            entry_lane_id: "west_0".to_owned(),
            exit_lane_id: "east_0".to_owned(),
            junction_id: "J".to_owned(),
            entry_end_m: [6806.88, 5727.52],
            weld_target_m: [6806.90, 5727.53],
            displacement_m: 0.0224,
            span_m: 0.3105,
            shape_len_m: 0.3105,
            locality_m: 0.0,
            controlled: true,
            shared_traversal_count: 0,
            shared_traversals: Vec::new(),
            detail: String::new(),
        }];
        let report =
            InfeasibilityReport::render(entries, ReportSource::unverified_unknown(), &records);
        assert!(report.rendered.contains("| 机制 | 预算裁决 | span |"));
        assert!(report.rendered.contains("采样候选耗尽"));
        assert!(report.rendered.contains("- 发射预算裁决分布"));
        assert!(
            report
                .rendered
                .contains("## 点状 stub 删焊处置（已归一化 / 拒绝保留）")
        );
        assert!(
            report
                .rendered
                .contains("| `:J_0_0` | `west_0` | `east_0` | `J` |")
        );
        assert!(report.rendered.contains("未支持-未评估"));
    }

    #[test]
    fn degenerate_centerline_synthesizes_micro_segment_along_clamps() {
        let points = [pt(5.0, 5.0), pt(5.0, 5.0)];
        let program = repair_curve(
            &points,
            Some([1.0, 0.0, 0.0]),
            Some([1.0, 0.0, 0.0]),
            None,
            None,
        )
        .expect("repair");
        assert_eq!(seg_ends(&program), 1);
        let re::RoadEditingCurveSegmentGeometry::Line { end } = program.segments()[0].geometry()
        else {
            panic!("micro segment must be a line");
        };
        assert!((end.x() - 5.0 - DEGENERATE_SEGMENT_METERS).abs() < 1e-9);
        assert!((end.y() - 5.0).abs() < 1e-9);
        // 不在任何 maneuver 路径中的零长边：fail-closed。
        assert!(repair_curve(&points, None, None, None, None).is_err());
    }
}
