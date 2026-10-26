//! #1084 S0-F：完整机动路径相邻对的跨边方向调查。
//!
//! 观察生产转换路径（`emit::observe`）记录的真实 `repair_curve` 发射结果与归一化
//! 后的完整机动路径，对 `entry + 全部 internal + exit` 的每个相邻位置建立
//! 路径位置计数，并按最终边身份有序二元组去重。仅当两侧都真实发射成功时，
//! 才把两侧原始程序原样放进最小合法编译夹具：前后各补一条与端弦同向的 2 m
//! 直线 approach 边，目标接缝、目标规范点与目标方向不移动、不重算、不重采样。
//! compiler 裁决只取自真实编译输出；本模块的弦长与角度只作观测。诊断模式的
//! 占位程序不进入任何检查。
//!
//! 判定顺序与 HIR 空间连接校验一致：先接缝位置（`DiscontinuousJoin`），位置通过
//! 后才检查方向（`DirectionDiscontinuity`）。编译在到达目标连接校验之前失败、
//! 或失败诊断不指向目标边对时，记为“未完成目标方向检查”。

use std::collections::{BTreeMap, HashMap};

use laneflow_compiler::road_editing as re;
use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, DiagnosticBundle, DiagnosticPayload,
    GeometryAccuracyProfile, GeometryDirectionProfile, SpatialGeometryViolation,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::{
    Vec3, add, angle_rad, classify_infeasible, point3, quantize_vec3, scale, sub, unit,
};
use super::witness::{FRAME_KEY, Stage, approach_declarations};
use crate::output::emit::observe::{EdgeEmission, EmissionObservation};
use crate::output::model::{ManeuverPath, Movement};

/// 夹具前后补边长度（米）。
const LEAD_METERS: f64 = 2.0;
const LEAD_IN_KEY: &str = "lead-in";
const LEFT_KEY: &str = "left";
const RIGHT_KEY: &str = "right";
const LEAD_OUT_KEY: &str = "lead-out";

/// 一条真实发射成功边的规范点（逐点 `f32` 量化）与 f64 首末点。
struct EdgeGeometry {
    start: Vec3,
    end: Vec3,
    points: Vec<[f32; 3]>,
}

fn vec3(point: re::RoadEditingPoint3) -> Vec3 {
    [point.x(), point.y(), point.z()]
}

/// 发射程序全部为 Line 段时返回其规范点；否则返回 `None`。
fn edge_geometry(program: &re::RoadEditingCurveProgram) -> Option<EdgeGeometry> {
    let start = vec3(program.start());
    let mut ends = Vec::with_capacity(program.segments().len());
    for segment in program.segments() {
        let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
            return None;
        };
        ends.push(vec3(end));
    }
    let end = *ends.last()?;
    let points = std::iter::once(start)
        .chain(ends)
        .map(quantize_vec3)
        .collect();
    Some(EdgeGeometry { start, end, points })
}

fn bits(point: [f32; 3]) -> [u32; 3] {
    point.map(f32::to_bits)
}

/// 完整程序的无损测试记录；不以段数和首末点代替内部几何。
fn program_record(program: &re::RoadEditingCurveProgram) -> Value {
    let point_bits = |point| vec3(point).map(f64::to_bits);
    let segments = program
        .segments()
        .iter()
        .map(|segment| {
            let geometry = match segment.geometry() {
                re::RoadEditingCurveSegmentGeometry::Line { end } => {
                    json!({"kind": "Line", "end_f64_bits": point_bits(end)})
                }
                re::RoadEditingCurveSegmentGeometry::CubicBezier {
                    control_1,
                    control_2,
                    end,
                } => {
                    json!({"kind": "CubicBezier", "control_1_f64_bits": point_bits(control_1),
                    "control_2_f64_bits": point_bits(control_2), "end_f64_bits": point_bits(end)})
                }
            };
            json!({"geometry": geometry, "canvas_selection": segment.canvas_selection()})
        })
        .collect::<Vec<_>>();
    json!({
        "start_f64_bits": point_bits(program.start()), "segments": segments,
        "line_points_f32_bits": edge_geometry(program).map(|geometry| {
            geometry.points.into_iter().map(bits).collect::<Vec<_>>()
        }),
    })
}

fn sampling_samples(observation: &EmissionObservation) -> String {
    use super::super::{repair_curve, sampling_observation};

    let selected = [
        "sumo::-10046_0_0",
        "sumo:--30452_0",
        "sumo::-10004_3_0",
        "sumo:--30316_0",
        "sumo::-10564_6_0",
        "sumo:--30358#12_1",
        "sumo::-1000_2_0",
        "sumo::-10004_5_1",
        "sumo:--30266_0",
        "sumo::-10004_0_0",
        "sumo:--30244#0_0",
        "sumo:--32668#0_0",
        "sumo::-19726_1_0",
        "sumo:-32668#0_0",
        "sumo::-25490_3_0",
        "sumo::-8018_0_1",
        "sumo:-32770#1_1",
    ];
    let mut rows = Vec::new();
    for ((edge, original), input) in observation.edges.iter().zip(&observation.inputs) {
        if !selected.contains(&edge.as_str()) {
            continue;
        }
        let (replay, calls) = sampling_observation::capture(|| {
            repair_curve(
                &input.points,
                input.clamps[0],
                input.clamps[1],
                input.clamp_sources[0].as_ref(),
                input.clamp_sources[1].as_ref(),
            )
        });
        let outcome = match (original, replay) {
            (EdgeEmission::Emitted(original), Ok(repaired)) => {
                assert_eq!(program_record(original), program_record(&repaired.program));
                json!({"status": "emitted", "program": program_record(&repaired.program),
                    "quantized_arc_f64_bits": repaired.quantized_arc_meters.to_bits()})
            }
            (EdgeEmission::Failed(original), Err(error)) => {
                assert_eq!(*original, error.to_string());
                json!({"status": "emission_failed", "error": original})
            }
            _ => panic!("sampling replay changed original outcome: {edge}"),
        };
        rows.push((edge, json!({"edge": edge, "original_replay_equal": true,
            "input_points_f64_bits": input.points.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>(),
            "clamps_f64_bits": input.clamps.map(|p| p.map(|p| p.map(f64::to_bits))),
            "outcome": outcome, "sample_cubic_calls": calls})));
    }
    assert_eq!(
        rows.len(),
        selected.len(),
        "all selected real samples observed"
    );
    rows.sort_by(|a, b| a.0.cmp(b.0));
    rows.into_iter()
        .map(|(_, row)| format!("{row}\n"))
        .collect()
}

/// 量化弦的 f64 促升单位方向（与 sink 同一算术）。
fn chord_dir(from: [f32; 3], to: [f32; 3]) -> Vec3 {
    unit([
        f64::from(to[0]) - f64::from(from[0]),
        f64::from(to[1]) - f64::from(from[1]),
        f64::from(to[2]) - f64::from(from[2]),
    ])
    .expect("nonzero quantized chord")
}

fn f32_distance(from: [f32; 3], to: [f32; 3]) -> f32 {
    (to[0] - from[0])
        .hypot(to[1] - from[1])
        .hypot(to[2] - from[2])
}

/// 目标边对的真实 compiler 裁决。
#[derive(Clone, Debug, PartialEq)]
enum TargetVerdict {
    /// 编译通过：目标接缝位置与方向均通过；`points_match` 为两侧冻结点与
    /// 发射规范点逐位相同。
    Accepted {
        points_match: bool,
    },
    JoinPositionFailed {
        diagnostic: String,
    },
    DirectionFailed {
        diagnostic: String,
    },
    /// 编译失败但没有指向目标边对的连接诊断。
    NotCompleted {
        stage: Stage,
        diagnostics: Vec<String>,
    },
}

impl TargetVerdict {
    fn label(&self) -> &'static str {
        match self {
            TargetVerdict::Accepted { .. } => "accepted",
            TargetVerdict::JoinPositionFailed { .. } => "join_position_failed",
            TargetVerdict::DirectionFailed { .. } => "direction_failed",
            TargetVerdict::NotCompleted { .. } => "target_check_not_completed",
        }
    }
}

fn diagnostic_text(diagnostic: &laneflow_compiler::Diagnostic) -> String {
    format!(
        "{} {:?}: {:?}",
        diagnostic.code().as_str(),
        diagnostic.stable_key(),
        diagnostic.payload()
    )
}

fn classify_rejection(stage: Stage, bundle: &DiagnosticBundle) -> TargetVerdict {
    if stage == Stage::Compile {
        for diagnostic in bundle.diagnostics() {
            if let DiagnosticPayload::InvalidSpatialGeometry {
                lane_edge_key,
                related_lane_edge_key: Some(related),
                violation,
                ..
            } = diagnostic.payload()
                && &**lane_edge_key == LEFT_KEY
                && &**related == RIGHT_KEY
            {
                match violation {
                    SpatialGeometryViolation::DiscontinuousJoin { .. } => {
                        return TargetVerdict::JoinPositionFailed {
                            diagnostic: diagnostic_text(diagnostic),
                        };
                    }
                    SpatialGeometryViolation::DirectionDiscontinuity { .. } => {
                        return TargetVerdict::DirectionFailed {
                            diagnostic: diagnostic_text(diagnostic),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    TargetVerdict::NotCompleted {
        stage,
        diagnostics: bundle.diagnostics().iter().map(diagnostic_text).collect(),
    }
}

fn line(from: Vec3, to: Vec3) -> re::RoadEditingCurveProgram {
    re::RoadEditingCurveProgram::try_new(
        point3(from).expect("lead start"),
        vec![re::RoadEditingCurveSegment::line(
            point3(to).expect("lead end"),
        )],
    )
    .expect("lead program")
}

/// 最小合法编译夹具：`lead-in → left → right → lead-out` 单路径，left/right 为
/// 原始发射程序（直接几何内部边），前后补边沿两侧量化端弦方向。
fn compile_pair(
    left: &re::RoadEditingCurveProgram,
    right: &re::RoadEditingCurveProgram,
    left_geometry: &EdgeGeometry,
    right_geometry: &EdgeGeometry,
) -> TargetVerdict {
    let lp = &left_geometry.points;
    let rp = &right_geometry.points;
    let first = chord_dir(lp[0], lp[1]);
    let last = chord_dir(rp[rp.len() - 2], rp[rp.len() - 1]);
    let lead_in = line(
        sub(left_geometry.start, scale(first, LEAD_METERS)),
        left_geometry.start,
    );
    let lead_out = line(
        right_geometry.end,
        add(right_geometry.end, scale(last, LEAD_METERS)),
    );

    let limits = CompileLimits::single_network_1m_v2();
    let header = re::RoadEditingModuleHeader::try_new(
        "laneflow/s0f-adjacency",
        "s0f-adjacency",
        Vec::new(),
        re::RoadEditingProvenance::generated(
            "laneflow-lust-converter-s0f",
            Sha256::digest(b"s0f-adjacency-config").into(),
            Sha256::digest(b"s0f-adjacency-balanced-5cm-2deg").into(),
            None,
            "repository:tools/laneflow-lust-converter",
        )
        .expect("provenance"),
    )
    .expect("header");
    let mut builder = re::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .expect("builder");
    let edge = |key: &str| re::LaneEdgeReference::local(key).expect("edge ref");
    let mut declarations = vec![re::RoadEditingDeclaration::CanonicalFrame(
        re::CanonicalFrameInput::try_new(FRAME_KEY).expect("frame"),
    )];
    let mut alignments = Vec::new();
    for (key, program) in [(LEAD_IN_KEY, lead_in), (LEAD_OUT_KEY, lead_out)] {
        let (alignment, approach) = approach_declarations(key, program);
        alignments.push(alignment);
        declarations.extend(approach);
    }
    for (key, program) in [(LEFT_KEY, left), (RIGHT_KEY, right)] {
        declarations.push(re::RoadEditingDeclaration::LaneEdge(
            re::LaneEdgeInput::try_new(key, 13.89, Vec::new(), Some(program.clone()))
                .expect("target edge"),
        ));
    }
    declarations.extend([
        re::RoadEditingDeclaration::Junction(
            re::JunctionInput::try_new(
                "j",
                vec![edge(LEAD_IN_KEY), edge(LEAD_OUT_KEY)],
                vec![edge(LEFT_KEY), edge(RIGHT_KEY)],
            )
            .expect("junction"),
        ),
        re::RoadEditingDeclaration::Movement(
            re::MovementInput::try_new(
                "m",
                re::JunctionReference::local("j").expect("junction ref"),
                "in-road",
                "out-road",
            )
            .expect("movement"),
        ),
        re::RoadEditingDeclaration::ManeuverPath(
            re::ManeuverPathInput::try_new(
                "path-0",
                re::MovementReference::owner_scoped(vec!["j".to_owned()], "m")
                    .expect("movement ref"),
                edge(LEAD_IN_KEY),
                vec![edge(LEFT_KEY), edge(RIGHT_KEY)],
                edge(LEAD_OUT_KEY),
            )
            .expect("path"),
        ),
    ]);
    for alignment in alignments {
        if let Err(bundle) = builder.add_alignment(alignment) {
            return classify_rejection(Stage::Declaration, &bundle);
        }
    }
    for declaration in declarations {
        if let Err(bundle) = builder.add_declaration(declaration) {
            return classify_rejection(Stage::Declaration, &bundle);
        }
    }
    let model = match builder.finish() {
        Ok(model) => model,
        Err(bundle) => return classify_rejection(Stage::Finish, &bundle),
    };
    let buffer = match re::RoadEditingSourceWriter::new(&limits).write(model) {
        Ok(buffer) => buffer,
        Err(bundle) => return classify_rejection(Stage::Write, &bundle),
    };
    let mut unit_builder = CompilationUnitBuilder::new(limits);
    let input = re::RoadEditingModuleInput::try_new("s0f-adjacency", buffer.as_bytes(), None)
        .expect("module input");
    if let Err(bundle) = unit_builder.add_road_editing_module(input) {
        return classify_rejection(Stage::ModuleAdmission, &bundle);
    }
    let compilation_unit = match unit_builder.build() {
        Ok(compilation_unit) => compilation_unit,
        Err(bundle) => return classify_rejection(Stage::UnitBuild, &bundle),
    };
    match Compiler::new().compile(compilation_unit) {
        Ok(output) => {
            let frozen = output
                .lir()
                .lane_edges()
                .map(|e| {
                    e.spatial_geometry()
                        .expect("spatial geometry")
                        .points()
                        .map(|p| bits([p.x, p.y, p.z]))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let present = |points: &[[f32; 3]]| {
                let expected = points.iter().copied().map(bits).collect::<Vec<_>>();
                frozen.contains(&expected)
            };
            TargetVerdict::Accepted {
                points_match: present(lp) && present(rp),
            }
        }
        Err(bundle) => classify_rejection(Stage::Compile, &bundle),
    }
}

/// 路径位置键：junction、movement、机动路径与零基相邻位置。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PositionKey {
    junction: String,
    movement: String,
    path: String,
    position: usize,
}

/// 一侧边的发射状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SideState {
    Emitted,
    EmissionFailed,
    NonLineGeometry,
    MissingRecord,
}

impl SideState {
    fn label(self) -> &'static str {
        match self {
            SideState::Emitted => "emitted",
            SideState::EmissionFailed => "emission_failed",
            SideState::NonLineGeometry => "non_line_geometry",
            SideState::MissingRecord => "missing_record",
        }
    }
}

/// 去重有向边对的结局。
#[derive(Clone, Debug, PartialEq)]
enum PairOutcome {
    NotChecked { reason: String },
    Compiled(TargetVerdict),
}

impl PairOutcome {
    fn label(&self) -> String {
        match self {
            PairOutcome::NotChecked { reason } => format!("not_checked:{reason}"),
            PairOutcome::Compiled(verdict) => format!("compiled:{}", verdict.label()),
        }
    }
}

struct PairRecord {
    left: String,
    right: String,
    positions: Vec<PositionKey>,
    left_state: SideState,
    right_state: SideState,
    observation: Value,
    outcome: PairOutcome,
}

struct SurveyResult {
    pairs: Vec<PairRecord>,
    /// (路径位置, 对应去重边对下标)，按位置键排序。
    positions: Vec<(PositionKey, usize)>,
}

fn side_state(edges: &HashMap<&str, &EdgeEmission>, id: &str) -> (SideState, Option<EdgeGeometry>) {
    match edges.get(id) {
        None => (SideState::MissingRecord, None),
        Some(EdgeEmission::Failed(_)) => (SideState::EmissionFailed, None),
        Some(EdgeEmission::Emitted(program)) => match edge_geometry(program) {
            Some(geometry) => (SideState::Emitted, Some(geometry)),
            None => (SideState::NonLineGeometry, None),
        },
    }
}

fn seam_observation(left: &EdgeGeometry, right: &EdgeGeometry) -> Value {
    let lp = &left.points;
    let rp = &right.points;
    let left_last = [lp[lp.len() - 2], lp[lp.len() - 1]];
    let right_first = [rp[0], rp[1]];
    json!({
        "left_last_chord_bits": left_last.map(bits),
        "right_first_chord_bits": right_first.map(bits),
        "left_last_chord_points": left_last,
        "right_first_chord_points": right_first,
        "seam_bitwise_equal": bits(left_last[1]) == bits(right_first[0]),
        "seam_gap_m_f32": f32_distance(left_last[1], right_first[0]),
        "left_last_chord_m_f32": f32_distance(left_last[0], left_last[1]),
        "right_first_chord_m_f32": f32_distance(right_first[0], right_first[1]),
        "observed_turn_deg": angle_rad(
            chord_dir(left_last[0], left_last[1]),
            chord_dir(right_first[0], right_first[1]),
        )
        .to_degrees(),
    })
}

/// 按固定键枚举全部相邻位置、去重，并对两侧都真实发射成功的边对做真实编译。
fn run_survey(observation: &EmissionObservation, workers: usize) -> SurveyResult {
    let junction_by_movement: HashMap<&str, &str> = observation
        .movements
        .iter()
        .map(|m| (m.id.as_str(), m.junction_id.as_str()))
        .collect();
    let edges: HashMap<&str, &EdgeEmission> = observation
        .edges
        .iter()
        .map(|(id, emission)| (id.as_str(), emission))
        .collect();
    assert_eq!(
        edges.len(),
        observation.edges.len(),
        "duplicate edge records"
    );

    let mut grouped: BTreeMap<(String, String), Vec<PositionKey>> = BTreeMap::new();
    for path in &observation.maneuver_paths {
        let chain = std::iter::once(&path.entry_edge_id)
            .chain(&path.internal_edge_ids)
            .chain(std::iter::once(&path.exit_edge_id))
            .collect::<Vec<_>>();
        for (position, window) in chain.windows(2).enumerate() {
            grouped
                .entry((window[0].clone(), window[1].clone()))
                .or_default()
                .push(PositionKey {
                    junction: junction_by_movement
                        .get(path.movement_id.as_str())
                        .expect("movement of path")
                        .to_string(),
                    movement: path.movement_id.clone(),
                    path: path.id.clone(),
                    position,
                });
        }
    }

    let mut pairs = grouped
        .into_iter()
        .map(|((left, right), mut positions)| {
            positions.sort();
            let (left_state, left_geometry) = side_state(&edges, &left);
            let (right_state, right_geometry) = side_state(&edges, &right);
            let observation = match (&left_geometry, &right_geometry) {
                (Some(l), Some(r)) => seam_observation(l, r),
                _ => Value::Null,
            };
            let outcome = match (left_state, right_state) {
                (SideState::Emitted, SideState::Emitted) => PairOutcome::NotChecked {
                    reason: "pending".to_owned(),
                },
                (l, r) if l == r => PairOutcome::NotChecked {
                    reason: format!("both_{}", l.label()),
                },
                (SideState::Emitted, r) => PairOutcome::NotChecked {
                    reason: format!("right_{}", r.label()),
                },
                (l, SideState::Emitted) => PairOutcome::NotChecked {
                    reason: format!("left_{}", l.label()),
                },
                (l, r) => PairOutcome::NotChecked {
                    reason: format!("left_{}_right_{}", l.label(), r.label()),
                },
            };
            PairRecord {
                left,
                right,
                positions,
                left_state,
                right_state,
                observation,
                outcome,
            }
        })
        .collect::<Vec<_>>();

    let eligible = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.left_state == SideState::Emitted && p.right_state == SideState::Emitted)
        .map(|(index, p)| (index, p.left.clone(), p.right.clone()))
        .collect::<Vec<_>>();
    let chunk = eligible.len().div_ceil(workers.max(1)).max(1);
    let verdicts = std::thread::scope(|scope| {
        let handles = eligible
            .chunks(chunk)
            .map(|work| {
                let edges = &edges;
                scope.spawn(move || {
                    work.iter()
                        .map(|(index, left, right)| {
                            let (EdgeEmission::Emitted(l), EdgeEmission::Emitted(r)) =
                                (edges[left.as_str()], edges[right.as_str()])
                            else {
                                unreachable!("eligible pairs are emitted on both sides");
                            };
                            let lg = edge_geometry(l).expect("line geometry");
                            let rg = edge_geometry(r).expect("line geometry");
                            (*index, compile_pair(l, r, &lg, &rg))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("survey worker"))
            .collect::<Vec<_>>()
    });
    for (index, verdict) in verdicts {
        pairs[index].outcome = PairOutcome::Compiled(verdict);
    }

    let mut positions = pairs
        .iter()
        .enumerate()
        .flat_map(|(index, p)| p.positions.iter().map(move |k| (k.clone(), index)))
        .collect::<Vec<_>>();
    positions.sort();
    SurveyResult { pairs, positions }
}

fn verdict_json(outcome: &PairOutcome) -> Value {
    match outcome {
        PairOutcome::NotChecked { reason } => json!({ "checked": false, "reason": reason }),
        PairOutcome::Compiled(verdict) => match verdict {
            TargetVerdict::Accepted { points_match } => json!({
                "checked": true, "compiler": "accepted", "frozen_points_match_emission": points_match,
            }),
            TargetVerdict::JoinPositionFailed { diagnostic } => json!({
                "checked": true, "compiler": "join_position_failed", "stage": "compile",
                "diagnostic": diagnostic,
            }),
            TargetVerdict::DirectionFailed { diagnostic } => json!({
                "checked": true, "compiler": "direction_failed", "stage": "compile",
                "diagnostic": diagnostic,
            }),
            TargetVerdict::NotCompleted { stage, diagnostics } => json!({
                "checked": false, "reason": "target_check_not_completed",
                "stage": stage.label(), "diagnostics": diagnostics,
            }),
        },
    }
}

/// 两类计数：按结局标签分组，并核对守恒。
fn counts(result: &SurveyResult) -> Value {
    let mut by_pair: BTreeMap<String, usize> = BTreeMap::new();
    for pair in &result.pairs {
        *by_pair.entry(pair.outcome.label()).or_default() += 1;
    }
    let mut by_position: BTreeMap<String, usize> = BTreeMap::new();
    for (_, index) in &result.positions {
        *by_position
            .entry(result.pairs[*index].outcome.label())
            .or_default() += 1;
    }
    let summarize = |groups: &BTreeMap<String, usize>, total: usize| {
        let compiled = groups
            .iter()
            .filter(|(k, _)| k.starts_with("compiled:"))
            .map(|(_, v)| v)
            .sum::<usize>();
        let not_checked = groups
            .iter()
            .filter(|(k, _)| k.starts_with("not_checked:"))
            .map(|(_, v)| v)
            .sum::<usize>();
        assert_eq!(compiled + not_checked, total, "count conservation");
        json!({
            "total": total,
            "compiled": compiled,
            "not_compiled": not_checked,
            "by_outcome": groups,
        })
    };
    json!({
        "path_positions": summarize(&by_position, result.positions.len()),
        "distinct_directed_pairs": summarize(&by_pair, result.pairs.len()),
    })
}

fn pair_json(
    pair: &PairRecord,
    sources: &HashMap<String, (String, String, bool)>,
    emission_errors: &HashMap<&str, &str>,
) -> Value {
    let source = |id: &str| {
        sources.get(id).map(|(lane, road, internal)| {
            json!({ "lane": lane, "road": road, "function_internal": internal })
        })
    };
    json!({
        "left": pair.left,
        "right": pair.right,
        "left_source": source(&pair.left),
        "right_source": source(&pair.right),
        "left_emission": pair.left_state.label(),
        "right_emission": pair.right_state.label(),
        "left_emission_error": emission_errors.get(pair.left.as_str()),
        "right_emission_error": emission_errors.get(pair.right.as_str()),
        "positions": pair.positions.iter().map(|k| json!({
            "junction": k.junction, "movement": k.movement, "path": k.path, "position": k.position,
        })).collect::<Vec<_>>(),
        "seam": pair.observation,
        "outcome": pair.outcome.label(),
        "verdict": verdict_json(&pair.outcome),
    })
}

#[test]
#[ignore = "requires LUST_SOURCE_DIR at c4bd5bd3 and S0F_OUTPUT_DIR (fresh directory)"]
fn pinned_lust_path_adjacency_survey() {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use crate::convert::profiles::{convert_vehicle_profiles, select_passenger_vtypes};
    use crate::convert::topology::{
        TopologyConvertOptions, convert_network_topology_with_tll_and_profiles,
    };
    use crate::source::pinned::{LUST_COMMIT, PINNED_SOURCE_FILES};
    use crate::source::verify::prepare_verified_lust_inputs;
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml};

    let started = Instant::now();
    let root = PathBuf::from(std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR"));
    let output_dir = PathBuf::from(std::env::var("S0F_OUTPUT_DIR").expect("S0F_OUTPUT_DIR"));
    if output_dir.exists() {
        assert!(
            fs::read_dir(&output_dir)
                .expect("read output dir")
                .next()
                .is_none(),
            "S0F_OUTPUT_DIR must be a fresh directory: {}",
            output_dir.display()
        );
    }
    fs::create_dir_all(&output_dir).expect("create output dir");

    // 与全量验收同一来源准备：revision + 全部 pinned 摘要，消费时重哈希。
    let prepared = prepare_verified_lust_inputs(&root).expect("verified LuST inputs");
    let network = parse_sumo_network_xml(prepared.net_xml()).expect("parse net");
    let tll = parse_tll_static_xml(prepared.tll_xml()).expect("parse tll");
    let vtypes = parse_vtypes_xml(prepared.vtypes_xml()).expect("parse vtypes");
    let profiles =
        convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).expect("passenger vtypes"))
            .expect("profiles");
    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    let (artifacts, observation) = crate::output::emit::observe::capture(|| {
        convert_network_topology_with_tll_and_profiles(
            &network,
            &tll,
            &profiles,
            &options,
            prepared.report_source().clone(),
        )
    });
    let artifacts = artifacts.expect("diagnostic conversion");
    let report = artifacts.infeasibility_report.as_ref().expect("report");
    // 观察的发射与检入清单是同一状态：失败集合计数一致、清单逐字节一致。
    let evidence = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md"),
    )
    .expect("committed survey");
    assert_eq!(report.rendered, evidence.replace("\r\n", "\n"));
    let failed = observation
        .edges
        .iter()
        .filter(|(_, e)| matches!(e, EdgeEmission::Failed(_)))
        .count();
    assert_eq!(failed, report.total());
    assert_eq!(
        observation.edges.len(),
        artifacts.counts.lane_edges as usize
    );

    let sources = network
        .lanes
        .iter()
        .map(|lane| {
            (
                lane.laneflow_id(),
                (
                    lane.id.clone(),
                    lane.edge_id.clone(),
                    lane.function_internal,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let emission_errors = observation
        .edges
        .iter()
        .filter_map(|(id, e)| match e {
            EdgeEmission::Failed(message) => Some((id.as_str(), message.as_str())),
            EdgeEmission::Emitted(_) => None,
        })
        .collect::<HashMap<_, _>>();
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let survey_started = Instant::now();
    let result = run_survey(&observation, workers);
    let survey_seconds = survey_started.elapsed().as_secs_f64();
    let counts = counts(&result);

    let summary = json!({
        "schema": "laneflow-1084-s0f-summary",
        "source": {
            "lust_commit": LUST_COMMIT,
            "net_digest": prepared.report_source().net_digest(),
            "pinned_files": PINNED_SOURCE_FILES.iter().map(|f| json!({
                "path": f.relative_path, "bytes": f.bytes, "sha256": f.sha256_hex,
            })).collect::<Vec<_>>(),
        },
        "geometry_profile": "Balanced5Cm + Balanced2Deg",
        "fixture": {
            "layout": "lead-in -> left -> right -> lead-out (left/right = emitted programs as direct-geometry internal edges)",
            "lead_meters": LEAD_METERS,
        },
        "edges": {
            "recorded": observation.edges.len(),
            "emitted": observation.edges.len() - failed,
            "emission_failed": failed,
        },
        "maneuver_paths": observation.maneuver_paths.len(),
        "counts": counts,
    });
    let mut pairs_text = String::new();
    for pair in &result.pairs {
        pairs_text.push_str(&pair_json(pair, &sources, &emission_errors).to_string());
        pairs_text.push('\n');
    }
    let mut positions_text = String::new();
    for (key, index) in &result.positions {
        let pair = &result.pairs[*index];
        positions_text.push_str(
            &json!({
                "junction": key.junction, "movement": key.movement, "path": key.path,
                "position": key.position, "left": pair.left, "right": pair.right,
                "outcome": pair.outcome.label(),
            })
            .to_string(),
        );
        positions_text.push('\n');
    }
    // 每条边的真实发射结果与 `repair_curve` 原样输入（S1 可复现样本来源）。
    assert_eq!(observation.inputs.len(), observation.edges.len());
    let mut edge_rows = observation
        .edges
        .iter()
        .zip(&observation.inputs)
        .map(|((id, emission), input)| {
            let (lane, road, internal) = sources.get(id).cloned().expect("source lane");
            let emission = match emission {
                EdgeEmission::Emitted(program) => {
                    let geometry = edge_geometry(program);
                    json!({
                        "status": if geometry.is_some() { "emitted" } else { "emitted_non_line" },
                        "segments": program.segments().len(),
                        "full_program": program_record(program),
                        "first_point_bits": geometry.as_ref().map(|g| bits(g.points[0])),
                        "last_point_bits": geometry.as_ref().map(|g| bits(g.points[g.points.len() - 1])),
                    })
                }
                EdgeEmission::Failed(message) => {
                    let diagnosis = classify_infeasible(id, message.clone(), internal);
                    json!({
                        "status": "emission_failed",
                        "budget_outcome": diagnosis.budget_outcome.label(),
                        "mechanism": diagnosis.mechanism.label(),
                        "error": message,
                    })
                }
            };
            (
                id.clone(),
                json!({
                    "edge": id, "lane": lane, "road": road, "function_internal": internal,
                    "input_points": input.points, "clamp_start": input.clamps[0],
                    "clamp_finish": input.clamps[1], "emission": emission,
                }),
            )
        })
        .collect::<Vec<_>>();
    edge_rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut edges_text = String::new();
    for (_, row) in &edge_rows {
        edges_text.push_str(&row.to_string());
        edges_text.push('\n');
    }
    fs::write(output_dir.join("edge-emissions.jsonl"), &edges_text).expect("edges");
    fs::write(
        output_dir.join("sampling-samples.jsonl"),
        sampling_samples(&observation),
    )
    .expect("sampling samples");
    let summary_text = serde_json::to_string_pretty(&summary).expect("summary json");
    fs::write(output_dir.join("summary.json"), format!("{summary_text}\n")).expect("summary");
    fs::write(output_dir.join("pairs.jsonl"), &pairs_text).expect("pairs");
    fs::write(output_dir.join("positions.jsonl"), &positions_text).expect("positions");
    let meta = json!({
        "unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs(),
        "total_seconds": started.elapsed().as_secs_f64(),
        "survey_compile_seconds": survey_seconds,
        "workers": workers,
        "source_dir_env": "LUST_SOURCE_DIR",
        "output_dir_env": "S0F_OUTPUT_DIR",
    });
    fs::write(
        output_dir.join("run-meta.json"),
        format!("{}\n", serde_json::to_string_pretty(&meta).expect("meta")),
    )
    .expect("meta");
    println!("S0F-SUMMARY {summary_text}");
}

// ---------------------------------------------------------------------------
// 普通小夹具：定位准确性与计数规则
// ---------------------------------------------------------------------------

#[test]
fn complete_program_record_detects_middle_point_change() {
    let first = program(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
    let changed = program(&[[0.0, 0.0, 0.0], [1.0, 0.01, 0.0], [2.0, 0.0, 0.0]]);
    assert_eq!(first.segments().len(), changed.segments().len());
    let first_points = edge_geometry(&first).unwrap().points;
    let changed_points = edge_geometry(&changed).unwrap().points;
    assert_eq!(first_points.first(), changed_points.first());
    assert_eq!(first_points.last(), changed_points.last());
    assert_ne!(program_record(&first), program_record(&changed));
}

/// 同参数 cubic 与量化弦之差仍是 cubic；控制多边形凸包给出保守距离上界。
/// 这覆盖弦内部，但该数值未被接受为 S1 的合同门槛。
fn cubic_chord_deviation_bound(control: [Vec3; 4], chord: [Vec3; 2]) -> f64 {
    let linear = [
        chord[0],
        scale(add(scale(chord[0], 2.0), chord[1]), 1.0 / 3.0),
        scale(add(chord[0], scale(chord[1], 2.0)), 1.0 / 3.0),
        chord[1],
    ];
    control
        .into_iter()
        .zip(linear)
        .map(|(p, q)| super::super::norm(sub(p, q)))
        .fold(0.0, f64::max)
}

#[test]
fn deviation_bound_covers_curve_interior_when_vertices_lie_on_curve() {
    use super::super::{cubic_at, norm};
    let control = [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let [a, c1, c2, b] = control;
    assert_eq!(cubic_at(a, c1, c2, b, 0.0), a);
    assert_eq!(cubic_at(a, c1, c2, b, 1.0), b);
    let midpoint_deviation = norm(sub(cubic_at(a, c1, c2, b, 0.5), [0.5, 0.0, 0.0]));
    assert_eq!(midpoint_deviation, 0.75);
    assert!(cubic_chord_deviation_bound(control, [a, b]) >= midpoint_deviation);
}

fn program(points: &[Vec3]) -> re::RoadEditingCurveProgram {
    re::RoadEditingCurveProgram::try_new(
        point3(points[0]).expect("start"),
        points[1..]
            .iter()
            .map(|&p| re::RoadEditingCurveSegment::line(point3(p).expect("end")))
            .collect(),
    )
    .expect("program")
}

fn heading(deg: f64) -> Vec3 {
    let r = deg.to_radians();
    [r.cos(), 0.0, r.sin()]
}

fn fixture_path(id: &str, entry: &str, internals: &[&str], exit: &str) -> ManeuverPath {
    ManeuverPath {
        id: id.to_owned(),
        movement_id: "m".to_owned(),
        entry_edge_id: entry.to_owned(),
        internal_edge_ids: internals.iter().map(|s| (*s).to_owned()).collect(),
        exit_edge_id: exit.to_owned(),
    }
}

fn fixture_observation(
    paths: Vec<ManeuverPath>,
    edges: Vec<(&str, EdgeEmission)>,
) -> EmissionObservation {
    EmissionObservation {
        maneuver_paths: paths,
        movements: vec![Movement {
            id: "m".to_owned(),
            junction_id: "J".to_owned(),
            from_road_edge_id: "a".to_owned(),
            to_road_edge_id: "b".to_owned(),
        }],
        edges: edges
            .into_iter()
            .map(|(id, e)| (id.to_owned(), e))
            .collect(),
        inputs: Vec::new(),
    }
}

fn emitted(points: &[Vec3]) -> EdgeEmission {
    EdgeEmission::Emitted(program(points))
}

fn pair<'a>(result: &'a SurveyResult, left: &str, right: &str) -> &'a PairRecord {
    result
        .pairs
        .iter()
        .find(|p| p.left == left && p.right == right)
        .expect("pair present")
}

/// 直线入口 (0,0)→(5,0)；内部边自 (5,0) 起沿 `deg` 方向两段各 1 m；出口沿末弦。
fn straight_chain(deg: f64) -> Vec<(&'static str, EdgeEmission)> {
    let s = [5.0, 0.0, 0.0];
    let m = add(s, heading(deg));
    let e = add(m, heading(deg));
    vec![
        ("entry", emitted(&[[0.0; 3], s])),
        ("internal", emitted(&[s, m, e])),
        ("exit", emitted(&[e, add(e, scale(heading(deg), 5.0))])),
    ]
}

#[test]
fn s0f_continuous_pairs_are_accepted() {
    let obs = fixture_observation(
        vec![fixture_path("p", "entry", &["internal"], "exit")],
        straight_chain(0.0),
    );
    let result = run_survey(&obs, 2);
    assert_eq!(result.pairs.len(), 2);
    assert_eq!(result.positions.len(), 2);
    for p in &result.pairs {
        assert_eq!(
            p.outcome,
            PairOutcome::Compiled(TargetVerdict::Accepted { points_match: true })
        );
    }
}

#[test]
fn s0f_seam_turn_over_two_degrees_is_localized_direction_failure() {
    let obs = fixture_observation(
        vec![fixture_path("p", "entry", &["internal"], "exit")],
        straight_chain(2.5),
    );
    let result = run_survey(&obs, 1);
    let target = pair(&result, "entry", "internal");
    let turn = target.observation["observed_turn_deg"]
        .as_f64()
        .expect("turn");
    assert!(turn > 2.0, "observed {turn}");
    assert!(
        target.observation["seam_bitwise_equal"]
            .as_bool()
            .expect("seam")
    );
    assert!(
        matches!(&target.outcome, PairOutcome::Compiled(TargetVerdict::DirectionFailed { diagnostic })
            if diagnostic.contains("lane_edge_key: \"left\", related_lane_edge_key: Some(\"right\")")),
        "{:?}",
        target.outcome
    );
    // 同一路径的另一个相邻对（内部边与出口同向）不被误判。
    assert_eq!(
        pair(&result, "internal", "exit").outcome,
        PairOutcome::Compiled(TargetVerdict::Accepted { points_match: true })
    );
}

#[test]
fn s0f_seam_gap_is_join_position_failure_not_direction_failure() {
    let mut edges = straight_chain(0.0);
    edges[1] = (
        "internal",
        emitted(&[[5.0, 0.0, 0.2], [6.0, 0.0, 0.2], [7.0, 0.0, 0.2]]),
    );
    edges[2] = ("exit", emitted(&[[7.0, 0.0, 0.2], [12.0, 0.0, 0.2]]));
    let obs = fixture_observation(
        vec![fixture_path("p", "entry", &["internal"], "exit")],
        edges,
    );
    let result = run_survey(&obs, 1);
    let target = pair(&result, "entry", "internal");
    assert!(
        !target.observation["seam_bitwise_equal"]
            .as_bool()
            .expect("seam")
    );
    assert!(
        matches!(&target.outcome, PairOutcome::Compiled(TargetVerdict::JoinPositionFailed { diagnostic })
            if diagnostic.contains("DiscontinuousJoin")),
        "{:?}",
        target.outcome
    );
}

#[test]
fn s0f_failed_edge_is_excluded_not_compiled() {
    let mut edges = straight_chain(0.0);
    edges[1] = (
        "internal",
        EdgeEmission::Failed("sampling exhausted".to_owned()),
    );
    let obs = fixture_observation(
        vec![fixture_path("p", "entry", &["internal"], "exit")],
        edges,
    );
    let result = run_survey(&obs, 1);
    assert_eq!(
        pair(&result, "entry", "internal").outcome,
        PairOutcome::NotChecked {
            reason: "right_emission_failed".to_owned()
        }
    );
    assert_eq!(
        pair(&result, "internal", "exit").outcome,
        PairOutcome::NotChecked {
            reason: "left_emission_failed".to_owned()
        }
    );
    assert!(pair(&result, "entry", "internal").observation.is_null());
    let counts = counts(&result);
    assert_eq!(counts["path_positions"]["compiled"], 0);
    assert_eq!(counts["path_positions"]["not_compiled"], 2);
}

#[test]
fn s0f_shared_pair_keeps_every_path_position() {
    let mut edges = straight_chain(0.0);
    let e = [7.0, 0.0, 0.0];
    edges.push(("exit2", emitted(&[e, [9.0, 0.0, 0.0]])));
    let obs = fixture_observation(
        vec![
            fixture_path("p0", "entry", &["internal"], "exit"),
            fixture_path("p1", "entry", &["internal"], "exit2"),
        ],
        edges,
    );
    let result = run_survey(&obs, 3);
    assert_eq!(result.positions.len(), 4);
    assert_eq!(result.pairs.len(), 3);
    let shared = pair(&result, "entry", "internal");
    assert_eq!(
        shared
            .positions
            .iter()
            .map(|k| k.path.as_str())
            .collect::<Vec<_>>(),
        ["p0", "p1"]
    );
    let counts = counts(&result);
    assert_eq!(counts["path_positions"]["total"], 4);
    assert_eq!(counts["distinct_directed_pairs"]["total"], 3);
}

#[test]
fn s0f_prerequisite_failure_is_not_counted_as_direction_failure() {
    // 内部边自身两段间转 2.5°（准入阶段 DirectionDiscontinuity），且目标接缝也转
    // 2.5°：编译在到达目标连接校验前失败，只能记为未完成目标方向检查。
    let s = [5.0, 0.0, 0.0];
    let m = add(s, heading(2.5));
    let e = add(m, heading(5.0));
    let edges = vec![
        ("entry", emitted(&[[0.0; 3], s])),
        ("internal", emitted(&[s, m, e])),
        ("exit", emitted(&[e, add(e, scale(heading(5.0), 5.0))])),
    ];
    let obs = fixture_observation(
        vec![fixture_path("p", "entry", &["internal"], "exit")],
        edges,
    );
    let result = run_survey(&obs, 1);
    let target = pair(&result, "entry", "internal");
    assert!(
        target.observation["observed_turn_deg"]
            .as_f64()
            .expect("turn")
            > 2.0
    );
    assert!(
        matches!(&target.outcome, PairOutcome::Compiled(TargetVerdict::NotCompleted { stage: Stage::ModuleAdmission, diagnostics })
            if diagnostics.iter().any(|d| d.contains("NumericFreeze(DirectionDiscontinuity)"))),
        "{:?}",
        target.outcome
    );
    assert_eq!(
        target.outcome.label(),
        "compiled:target_check_not_completed"
    );
}
