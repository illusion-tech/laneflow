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
use std::sync::Mutex;

use laneflow_compiler::road_editing as re;

use crate::{
    Error, Result,
    output::model::{SpatialEdge, TrafficPackage},
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
    /// 单行规范诊断：span j/n、弦长、单片/均转角、坐标、start/finish 切向来源。
    pub entry: String,
}

/// 收集 emission 层不可行诊断（lane 处理顺序即记录顺序，确定性）。
static INFEASIBLE_DIAGNOSTICS: Mutex<Vec<InfeasibilityDiagnosis>> = Mutex::new(Vec::new());

/// 记录一条不可行诊断（调用方已带 lane id 与 span 上下文）。
pub(crate) fn record_infeasible(lane_id: &str, entry: String) {
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
    INFEASIBLE_DIAGNOSTICS
        .lock()
        .expect("infeasible diagnostics lock poisoned")
        .push(InfeasibilityDiagnosis {
            lane_id: lane_id.to_owned(),
            is_internal: is_internal_lane(lane_id),
            junction,
            mechanism,
            entry,
        });
}

/// 取走全部诊断并清空。
pub(crate) fn drain_infeasible_diagnostics() -> Vec<InfeasibilityDiagnosis> {
    INFEASIBLE_DIAGNOSTICS
        .lock()
        .expect("infeasible diagnostics lock poisoned")
        .drain(..)
        .collect()
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
fn is_internal_lane(lane_id: &str) -> bool {
    lane_id
        .strip_prefix(SUMO_ID_PREFIX)
        .is_some_and(|rest| rest.starts_with(':'))
}

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
        .or_else(|| entry.split_once("turns ").and_then(|(_, tail)| between(tail, "", " deg >")))
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

/// 确定性诊断清单（验收重划后的正式交付物，见 #253 G1 补充记录）：
/// 同一输入下两次运行的 `rendered` 逐字节一致。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InfeasibilityReport {
    pub entries: Vec<InfeasibilityDiagnosis>,
    /// 逐字节稳定的 Markdown 渲染（诊断清单交付格式）。
    pub rendered: String,
}

impl InfeasibilityReport {
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn internal_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_internal).count()
    }

    pub fn external_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_internal).count()
    }

    pub fn mechanism_count(&self, mechanism: InfeasibilityMechanism) -> usize {
        self.entries.iter().filter(|e| e.mechanism == mechanism).count()
    }

    /// 内车道子集的机制计数（普查锁定的口径；authored 边不计入）。
    pub fn internal_mechanism_count(&self, mechanism: InfeasibilityMechanism) -> usize {
        self.entries
            .iter()
            .filter(|e| e.is_internal && e.mechanism == mechanism)
            .count()
    }

    pub fn junction_count(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|e| e.junction.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// 由诊断记录渲染确定性 Markdown 清单。
    pub fn render(entries: Vec<InfeasibilityDiagnosis>) -> Self {
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
        out.push_str("- 基线源：pinned c4bd5bd3 原样（无本地补丁）；`scenario/lust.net.xml` SHA-256 为\n");
        out.push_str("  `6f5d76223cf14b797ae6267f13b23eb6c872d76adec1fb22a8569a806dc09341`。\n");
        out.push_str("- 语义：发射层（CR 平滑 + 边界钳制 + 倒圆 + 采样）在本套验收常数\n");
        out.push_str("  （Balanced2Deg 全角 2°、HIR 退化段 0.1 m、发射弦长下限 0.105 m、\n");
        out.push_str("  f32 端点量化）下无法给出可发射几何的 lane 全集；逐条含 span、弦长、\n");
        out.push_str("  单片/均转角、坐标、切向来源与机制分类。\n");
        out.push_str("- 生成：converter 诊断模式（`TopologyConvertOptions::emit_infeasibility_report`）；\n");
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
        out.push_str("| # | lane | 类别 | junction | 机制 | span | chord(m) | 单片/均转(°) | 坐标 | start 切向 | finish 切向 |\n");
        out.push_str("| ---: | --- | --- | --- | --- | --- | ---: | ---: | --- | --- | --- |\n");
        for (index, e) in entries.iter().enumerate() {
            let i = index + 1;
            let (span, chord, turn, coord, start, finish) = parse_diagnosis_fields(&e.entry);
            let cls = if e.is_internal { "SUMO 内车道" } else { "authored 边" };
            let junction = e.junction.as_deref().unwrap_or(if e.is_internal { "(节点簇)" } else { "-" });
            out.push_str(&format!(
                "| {i} | `{}` | {cls} | {junction} | {} | {span} | {chord} | {turn} | ({coord}) | {start} | {finish} |\n",
                e.lane_id,
                e.mechanism.label(),
            ));
        }
        Self { entries, rendered: out }
    }
}

/// 零长内部 lane 的合成微段长度（米）：取采样弦长下限（> HIR 冻结的
/// 0.1 m 退化段下限，`SPATIAL_MIN_SEGMENT_LENGTH_METERS`），方向取边界钳制，
/// 方向连续性由构造保证。
const DEGENERATE_SEGMENT_METERS: f64 = SAMPLE_MIN_CHORD_METERS;

/// 软/硬折点分界（度）：≤ 30° 视为采样伪影走平滑；> 30° 视为真实硬角走倒圆。
const SOFT_KINK_MAX_DEG: f64 = 30.0;
/// 倒圆半径上限（米，G1 修订冻结值）：90° 硬角时偏离原折点 ≤ 5.0 m。
const FILLET_RADIUS_MAX_METERS: f64 = 5.0;
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
    Ok(re::RoadEditingPoint3::try_new(value[0], value[1], value[2])?)
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
        Side::Start => points
            .windows(2)
            .find_map(|w| unit(sub(w[1], w[0]))),
        Side::Finish => points
            .windows(2)
            .rev()
            .find_map(|w| unit(sub(w[1], w[0]))),
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
        ends.entry((a.to_owned(), Side::Finish)).or_default().push(index);
        ends.entry((b.to_owned(), Side::Start)).or_default().push(index);
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
            let tan_half = (theta * 0.5).tan();
            let d_limit = FILLET_CUTBACK_FRACTION * chord_lens[i - 1].min(chord_lens[i]);
            let radius = (d_limit / tan_half).min(FILLET_RADIUS_MAX_METERS);
            if radius > 1e-6 {
                let d = radius * tan_half;
                let t1 = add(v, scale(u, -d));
                let t2 = add(v, scale(w, d));
                // 圆弧的三次 Bezier 拟合：控制柄 (4/3)·r·tan(θ/4)，端点严格相切。
                let handle = (4.0 / 3.0) * radius * (theta * 0.25).tan();
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
/// HIR 冻结的退化段下限（米）：`SPATIAL_MIN_SEGMENT_LENGTH_METERS`，
/// 长度 ≤ 该值的规范段被拒绝。发射时按同一算术（f32 差分 + hypot）预检。
const QUANTIZED_MIN_SEGMENT_METERS: f32 = 0.1;

/// 发射水槽：持有已发射的 Line 链，并在每段入链前按 compiler 的验收算术
/// （端点 f32 量化 → 弦长 / 相邻弦全角）做确定性预检。
///
/// compiler 侧对应检查：
/// - HIR 冻结 `DegenerateSegment`：量化后弦长 ≤ 0.1 m 拒绝（spatial_freeze.rs）；
/// - road editing `DirectionDiscontinuity`：相邻量化弦全角 > 2°（Balanced2Deg）
///   拒绝（validate_canonical_polyline）。
/// 预检逐位复刻这两条（f32 差分 + hypot / f64 促升后夹角），失败即 fail-closed
/// 报量化坐标与实测值——该处几何在本套验收常数下不可行，诚实亮诊断。
struct EmissionSink {
    segments: Vec<re::RoadEditingCurveSegment>,
    /// 上一条已发射弦的量化终点（即程序量化首点或上一段终点）。
    prev_point: [f32; 3],
    /// 上一条已发射弦的量化方向（f64 促升）。
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
fn tangent_dir(
    spans: &[Span],
    j: usize,
    side: Side,
    clamp: Option<Vec3>,
    chord: Vec3,
) -> Vec3 {
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
        if let Some(dir) = clamp {
            if angle_rad(dir, chord) <= CLAMP_GUARD_RAD {
                return dir;
            }
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

/// de Casteljau 细分采样三次 Bezier 为 Line 链。
///
/// 片数按局部几何一次选定（等 t 参数切 n 片），而不是递归二分：二分网格
///（弦长逐次减半）会跨过"弦长下限 0.105m 与 weld 全角预算夹出的可行窗口"
/// ——例如 r=4.5m 倒圆的可行弦长窗口 (0.1, ~0.14]m，二分序列 …0.397 → 0.199
/// → 0.099 恰好全部落在窗外。等 n 切分可命中任意窗口；非均匀曲率造成的单片
/// 超差由递归重估（每片重新走一遍本函数）与 sink 的量化终检兜底。
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
    let n_floor = (chord_len / SAMPLE_MIN_CHORD_METERS).floor();
    if n_floor < 2.0 {
        // 再切必破弦长下限：单片在安全转角内则发射（sink 量化终检），否则该处
        // 几何在本套验收常数下不可行，fail-closed 报精确位置。
        if turn <= WELD_SAFE_TURN_RAD {
            return sink.push(b);
        }
        return Err(Error::SumoModel(format!(
            "repaired curve curvature is infeasible under the quantized weld budget: \
             chord {chord_len:.4} m turns {:.2} deg > {:.2} deg at ({}, {}, {}); \
             splitting finer would breach the {SAMPLE_MIN_CHORD_METERS} m chord floor",
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
    // 等 t 参数切 n 片：每次从剩余段切下 1/(剩余片数)。
    let n = n as u32;
    let mut rem = (a, c1, c2, dir_a);
    for k in 1..n {
        let t = 1.0 / f64::from(n - k + 1);
        let lerp = |p: Vec3, q: Vec3| add(p, scale(sub(q, p), t));
        let p01 = lerp(rem.0, rem.1);
        let p12 = lerp(rem.1, rem.2);
        let p23 = lerp(rem.2, b);
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
    sample_cubic(rem.0, rem.1, rem.2, b, rem.3, dir_b, sink)
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
        let err = repair_curve(&points, None, None, None, None).expect_err("tight fillet must fail");
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
        let program = repair_curve(&points, None, Some([1.0, 0.0, 0.0]), None, None).expect("repair");
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
        let program = repair_curve(&points, None, Some([-1.0, 0.0, 0.0]), None, None).expect("repair");
        assert!(matches!(
            program.segments()[0].geometry(),
            re::RoadEditingCurveSegmentGeometry::Line { .. }
        ));
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
            message.contains("boundary clamp (fan-out mean over maneuver boundary pairs [sibling->exit])"),
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
