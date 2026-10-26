//! #790 S0：两段几何见证的分层判定证据。
//!
//! 同一组候选规范点分别交给三层判定者：发射 sink 的保守边内预检（量化弦长
//! 须大于 0.1 m、相邻弦 ≤ 1.95°）、compiler 边内硬门（入/出口边与候选首末弦同向的
//! 隔离夹具）和完整 entry/internal/exit 路径编译（入/出口边沿声明切向）。
//! 只验证 `sink 接受 ⇒ compiler 边内接受`，不要求逆命题；compiler 结论只取自
//! 真实编译输出，本模块的量化观测只作解释。sink 拒绝时 compiler 仍收到从原始
//! 候选独立构造的完整程序。每个用例以 `S0-JSON ` 前缀输出一行 JSON 记录。

use laneflow_compiler::road_editing as re;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, DiagnosticBundle,
    GeometryAccuracyProfile, GeometryDirectionProfile,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::{
    EmissionSink, Vec3, add, angle_rad, point3, quantize_vec3, quantized_chord_meters, scale, sub,
    unit,
};

/// `quality_floor_precheck_is_not_proven_conflict` 的 L/3 控制柄 cubic（原点）。
const WITNESS_CUBIC: [Vec3; 4] = [
    [0.0, 0.0, 0.0],
    [0.06887035952165224, 0.0, 0.0],
    [0.1377765790067887, 0.0, 0.002225658258883879],
    [0.20650338640946556, 0.0, 0.006670021529027181],
];

/// 夹具入/出口直线边长度（米）：远离 0.1 m 退化下限与 100 mm 边长下限。
const LEAD_METERS: f64 = 2.0;
pub(super) const FRAME_KEY: &str = "s0-frame";
const ENTRY_KEY: &str = "entry";
const INTERNAL_KEY: &str = "internal";
const EXIT_KEY: &str = "exit";

/// LuST 规范坐标域：source `convBoundary="0.00,0.00,13613.76,11455.04"` 经
/// `(x, z) = (sx - 6806.88, sy - 5727.52)`（real-road-workloads §3.2）得到
/// x ∈ [-6806.88, 6806.88]、z ∈ [-5727.52, 5727.52]。
const LUST_HALF_X: f64 = 6_806.88;
const LUST_HALF_Z: f64 = 5_727.52;

/// 一个待判定候选：内部边规范点序列（f64，未量化）与完整路径的声明切向。
struct Candidate {
    case: &'static str,
    sample: String,
    offset: Vec3,
    internal: Vec<Vec3>,
    dir_in: Vec3,
    dir_out: Vec3,
}

fn lerp(p: Vec3, q: Vec3, t: f64) -> Vec3 {
    add(p, scale(sub(q, p), t))
}

/// 平移后的见证 cubic 在 t = 0.5 处的 de Casteljau 两段划分 `[a, m, b]`。
fn witness_candidate(case: &'static str, sample: &str, offset: Vec3) -> Candidate {
    let [a, c1, c2, b] = WITNESS_CUBIC.map(|p| add(p, offset));
    let (p01, p12, p23) = (lerp(a, c1, 0.5), lerp(c1, c2, 0.5), lerp(c2, b, 0.5));
    let mid = lerp(lerp(p01, p12, 0.5), lerp(p12, p23, 0.5), 0.5);
    Candidate {
        case,
        sample: sample.to_owned(),
        offset,
        internal: vec![a, mid, b],
        dir_in: [1.0, 0.0, 0.0],
        dir_out: unit(sub(b, c2)).expect("witness end tangent"),
    }
}

/// 原点起、两条 `chord` 米长弦、相邻弦在 XZ 平面内转 `turn_deg` 的折线候选；
/// 完整路径的声明切向即两条弦方向。
fn bend_candidate(case: &'static str, sample: &str, chord: f64, turn_deg: f64) -> Candidate {
    let turn = turn_deg.to_radians();
    let dir_out = [turn.cos(), 0.0, turn.sin()];
    let p1 = [chord, 0.0, 0.0];
    Candidate {
        case,
        sample: sample.to_owned(),
        offset: [0.0; 3],
        internal: vec![[0.0; 3], p1, add(p1, scale(dir_out, chord))],
        dir_in: [1.0, 0.0, 0.0],
        dir_out,
    }
}

/// 量化规范点上的边内观测：与 sink/HIR 同一算术的弦长与相邻弦夹角。
struct Observation {
    points: Vec<[f32; 3]>,
    chords: Vec<f32>,
    turns_deg: Vec<f64>,
}

/// 量化弦的 f64 促升单位方向（与 sink 同一算术）。
fn promote_delta(from: [f32; 3], to: [f32; 3]) -> Vec3 {
    unit([
        f64::from(to[0]) - f64::from(from[0]),
        f64::from(to[1]) - f64::from(from[1]),
        f64::from(to[2]) - f64::from(from[2]),
    ])
    .expect("nonzero quantized chord")
}

fn observe(points: &[Vec3]) -> Observation {
    let quantized = points
        .iter()
        .copied()
        .map(quantize_vec3)
        .collect::<Vec<_>>();
    let chords = quantized
        .windows(2)
        .map(|w| quantized_chord_meters(w[0], w[1]))
        .collect();
    let deltas = quantized
        .windows(2)
        .map(|w| promote_delta(w[0], w[1]))
        .collect::<Vec<_>>();
    let turns_deg = deltas
        .windows(2)
        .map(|w| angle_rad(w[0], w[1]).to_degrees())
        .collect();
    Observation {
        points: quantized,
        chords,
        turns_deg,
    }
}

fn bits(point: [f32; 3]) -> [u32; 3] {
    point.map(f32::to_bits)
}

#[derive(Debug, PartialEq)]
enum SinkVerdict {
    Accepted,
    /// `push_index` 为被拒弦在候选中的零基序号。
    Rejected {
        push_index: usize,
        message: String,
    },
}

fn sink_verdict(points: &[Vec3]) -> SinkVerdict {
    let mut sink = EmissionSink::new(points[0]);
    for (push_index, &end) in points[1..].iter().enumerate() {
        if let Err(error) = sink.push(end) {
            return SinkVerdict::Rejected {
                push_index,
                message: error.to_string(),
            };
        }
    }
    SinkVerdict::Accepted
}

/// compiler 拒绝时所在的公开受检入口。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Declaration,
    Finish,
    Write,
    ModuleAdmission,
    UnitBuild,
    Compile,
}

impl Stage {
    pub(super) fn label(self) -> &'static str {
        match self {
            Stage::Declaration => "builder-declaration",
            Stage::Finish => "builder-finish",
            Stage::Write => "source-write",
            Stage::ModuleAdmission => "module-admission",
            Stage::UnitBuild => "unit-build",
            Stage::Compile => "compile",
        }
    }
}

struct CompiledEdge {
    points: Vec<[f32; 3]>,
    length_mm: u32,
    arc_length_meters: f32,
}

struct CompiledPath {
    entry: CompiledEdge,
    internal: CompiledEdge,
    exit: CompiledEdge,
}

enum CompileVerdict {
    Accepted(CompiledPath),
    Rejected {
        stage: Stage,
        diagnostics: Vec<String>,
    },
}

impl CompileVerdict {
    fn accepted(&self) -> bool {
        matches!(self, CompileVerdict::Accepted(_))
    }

    fn rejected_with(&self, stage: Stage, needle: &str) -> bool {
        matches!(
            self,
            CompileVerdict::Rejected { stage: s, diagnostics }
                if *s == stage && diagnostics.iter().any(|d| d.contains(needle))
        )
    }
}

fn rejected(stage: Stage, bundle: &DiagnosticBundle) -> CompileVerdict {
    CompileVerdict::Rejected {
        stage,
        diagnostics: bundle
            .diagnostics()
            .iter()
            .map(|d| {
                format!(
                    "{} {:?}: {:?}",
                    d.code().as_str(),
                    d.stable_key(),
                    d.payload()
                )
            })
            .collect(),
    }
}

fn line_program(points: &[Vec3]) -> re::RoadEditingCurveProgram {
    re::RoadEditingCurveProgram::try_new(
        point3(points[0]).expect("start point"),
        points[1..]
            .iter()
            .map(|&p| re::RoadEditingCurveSegment::line(point3(p).expect("line end")))
            .collect(),
    )
    .expect("line program")
}

/// 与 converter 发射同构的 approach 边声明：alignment → corridor → section → lane。
pub(super) fn approach_declarations(
    key: &str,
    program: re::RoadEditingCurveProgram,
) -> (re::RoadAlignmentInput, Vec<re::RoadEditingDeclaration>) {
    let corridor_key = format!("{key}.road");
    let section = re::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")
        .expect("section ref");
    let lane = re::AuthoringLaneReference::owner_scoped(
        vec![corridor_key.clone(), "section".to_owned()],
        "lane",
    )
    .expect("lane ref");
    let alignment = re::RoadAlignmentInput::try_new(
        key,
        re::CanonicalFrameReference::local(FRAME_KEY).expect("frame ref"),
        program,
    )
    .expect("alignment");
    let declarations = vec![
        re::RoadEditingDeclaration::RoadCorridor(
            re::RoadCorridorInput::try_new(
                &corridor_key,
                re::RoadAlignmentReference::try_new(key).expect("alignment ref"),
                0.0,
                re::RoadEditingStationEnd::AlignmentEnd,
                section.clone(),
                lane.clone(),
                vec![re::RoadEditingCorridorElement::RoadSection(section.clone())],
            )
            .expect("corridor"),
        ),
        re::RoadEditingDeclaration::RoadSection(
            re::RoadSectionInput::try_new(
                "section",
                "motorLane",
                vec![lane.clone()],
                re::RoadCorridorReference::local(&corridor_key).expect("corridor ref"),
            )
            .expect("section"),
        ),
        re::RoadEditingDeclaration::AuthoringLane(
            re::AuthoringLaneInput::try_new(
                "lane",
                re::LaneEdgeReference::local(key).expect("edge ref"),
                re::RoadEditingLaneDirection::Forward,
                re::LinearWidthProfile::try_new(3.2, 3.2).expect("width"),
                None,
                section,
            )
            .expect("lane"),
        ),
        re::RoadEditingDeclaration::LaneEdge(
            re::LaneEdgeInput::try_new(key, 13.89, Vec::new(), None).expect("approach edge"),
        ),
    ];
    (alignment, declarations)
}

/// 构造 `entry → internal → exit` 单路径最小模块，走 converter 同档位
/// （Balanced5Cm + Balanced2Deg）的真实受检入口，逐阶段记录拒绝。
fn compile_path(internal: &[Vec3], lead_in: [Vec3; 2], lead_out: [Vec3; 2]) -> CompileVerdict {
    let limits = CompileLimits::single_network_1m_v2();
    let header = re::RoadEditingModuleHeader::try_new(
        "laneflow/s0-witness",
        "s0-witness",
        Vec::new(),
        re::RoadEditingProvenance::generated(
            "laneflow-lust-converter-s0",
            Sha256::digest(b"s0-witness-config").into(),
            Sha256::digest(b"s0-witness-balanced-5cm-2deg").into(),
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
    for (key, points) in [(ENTRY_KEY, lead_in), (EXIT_KEY, lead_out)] {
        let (alignment, approach) = approach_declarations(key, line_program(&points));
        alignments.push(alignment);
        declarations.extend(approach);
    }
    declarations.extend([
        re::RoadEditingDeclaration::LaneEdge(
            re::LaneEdgeInput::try_new(
                INTERNAL_KEY,
                13.89,
                Vec::new(),
                Some(line_program(internal)),
            )
            .expect("internal edge"),
        ),
        re::RoadEditingDeclaration::Junction(
            re::JunctionInput::try_new(
                "j",
                vec![edge(ENTRY_KEY), edge(EXIT_KEY)],
                vec![edge(INTERNAL_KEY)],
            )
            .expect("junction"),
        ),
        re::RoadEditingDeclaration::Movement(
            re::MovementInput::try_new(
                "m",
                re::JunctionReference::local("j").expect("junction ref"),
                "entry-road",
                "exit-road",
            )
            .expect("movement"),
        ),
        re::RoadEditingDeclaration::ManeuverPath(
            re::ManeuverPathInput::try_new(
                "path-0",
                re::MovementReference::owner_scoped(vec!["j".to_owned()], "m")
                    .expect("movement ref"),
                edge(ENTRY_KEY),
                vec![edge(INTERNAL_KEY)],
                edge(EXIT_KEY),
            )
            .expect("path"),
        ),
    ]);
    for alignment in alignments {
        if let Err(bundle) = builder.add_alignment(alignment) {
            return rejected(Stage::Declaration, &bundle);
        }
    }
    for declaration in declarations {
        if let Err(bundle) = builder.add_declaration(declaration) {
            return rejected(Stage::Declaration, &bundle);
        }
    }
    let model = match builder.finish() {
        Ok(model) => model,
        Err(bundle) => return rejected(Stage::Finish, &bundle),
    };
    let buffer = match re::RoadEditingSourceWriter::new(&limits).write(model) {
        Ok(buffer) => buffer,
        Err(bundle) => return rejected(Stage::Write, &bundle),
    };
    let mut unit = CompilationUnitBuilder::new(limits);
    let input = re::RoadEditingModuleInput::try_new("s0-witness", buffer.as_bytes(), None)
        .expect("module input");
    if let Err(bundle) = unit.add_road_editing_module(input) {
        return rejected(Stage::ModuleAdmission, &bundle);
    }
    let unit = match unit.build() {
        Ok(unit) => unit,
        Err(bundle) => return rejected(Stage::UnitBuild, &bundle),
    };
    match Compiler::new().compile(unit) {
        Ok(output) => CompileVerdict::Accepted(compiled_path(&output, internal, lead_in, lead_out)),
        Err(bundle) => rejected(Stage::Compile, &bundle),
    }
}

/// 按冻结首点与点数在 LIR 中取回三条边（LIR 按稳定身份排序，不按声明顺序）。
fn compiled_path(
    output: &CompilationOutput,
    internal: &[Vec3],
    lead_in: [Vec3; 2],
    lead_out: [Vec3; 2],
) -> CompiledPath {
    let edges = output
        .lir()
        .lane_edges()
        .map(|edge| {
            let geometry = edge.spatial_geometry().expect("spatial geometry");
            CompiledEdge {
                points: geometry.points().map(|p| [p.x, p.y, p.z]).collect(),
                length_mm: edge.length_mm(),
                arc_length_meters: geometry.arc_length_meters(),
            }
        })
        .collect::<Vec<_>>();
    let take = |points: &[Vec3]| -> CompiledEdge {
        let first = bits(quantize_vec3(points[0]));
        let index = edges
            .iter()
            .position(|e| e.points.len() == points.len() && bits(e.points[0]) == first)
            .expect("compiled edge by first point");
        let e = &edges[index];
        CompiledEdge {
            points: e.points.clone(),
            length_mm: e.length_mm,
            arc_length_meters: e.arc_length_meters,
        }
    };
    CompiledPath {
        entry: take(&lead_in),
        internal: take(internal),
        exit: take(&lead_out),
    }
}

/// 隔离 compiler 边内硬门：入/出口边与候选量化首末弦同向，跨边转角近零。
fn compile_edge_interior(candidate: &Candidate) -> CompileVerdict {
    let q = observe(&candidate.internal).points;
    let first = promote_delta(q[0], q[1]);
    let last = promote_delta(q[q.len() - 2], q[q.len() - 1]);
    let (start, end) = (
        candidate.internal[0],
        candidate.internal[candidate.internal.len() - 1],
    );
    compile_path(
        &candidate.internal,
        [sub(start, scale(first, LEAD_METERS)), start],
        [end, add(end, scale(last, LEAD_METERS))],
    )
}

/// 完整路径：入/出口边沿候选声明切向。
fn compile_full_path(candidate: &Candidate) -> CompileVerdict {
    let (start, end) = (
        candidate.internal[0],
        candidate.internal[candidate.internal.len() - 1],
    );
    compile_path(
        &candidate.internal,
        [sub(start, scale(candidate.dir_in, LEAD_METERS)), start],
        [end, add(end, scale(candidate.dir_out, LEAD_METERS))],
    )
}

fn turn_between(left: &[[f32; 3]], right: &[[f32; 3]]) -> f64 {
    let n = left.len();
    angle_rad(
        promote_delta(left[n - 2], left[n - 1]),
        promote_delta(right[0], right[1]),
    )
    .to_degrees()
}

fn verdict_json(verdict: &CompileVerdict, expected_internal: &[[f32; 3]]) -> Value {
    match verdict {
        CompileVerdict::Accepted(path) => json!({
            "result": "accepted",
            "internal_points_bits": path.internal.points.iter().copied().map(bits).collect::<Vec<_>>(),
            "internal_points_match_candidate": path.internal.points.iter().copied().map(bits)
                .eq(expected_internal.iter().copied().map(bits)),
            "internal_length_mm": path.internal.length_mm,
            "internal_arc_length_m": path.internal.arc_length_meters,
            "entry_length_mm": path.entry.length_mm,
            "exit_length_mm": path.exit.length_mm,
            "entry_seam_bitwise": bits(path.entry.points[path.entry.points.len() - 1])
                == bits(path.internal.points[0]),
            "exit_seam_bitwise": bits(path.internal.points[path.internal.points.len() - 1])
                == bits(path.exit.points[0]),
            "entry_seam_turn_deg": turn_between(&path.entry.points, &path.internal.points),
            "exit_seam_turn_deg": turn_between(&path.internal.points, &path.exit.points),
        }),
        CompileVerdict::Rejected { stage, diagnostics } => json!({
            "result": "rejected",
            "stage": stage.label(),
            "diagnostics": diagnostics,
        }),
    }
}

/// 一个候选的完整三层记录。
struct Record {
    observation: Observation,
    sink: SinkVerdict,
    edge_interior: CompileVerdict,
    full_path: CompileVerdict,
}

fn evaluate(candidate: &Candidate) -> Record {
    let record = Record {
        observation: observe(&candidate.internal),
        sink: sink_verdict(&candidate.internal),
        edge_interior: compile_edge_interior(candidate),
        full_path: compile_full_path(candidate),
    };
    let sink = match &record.sink {
        SinkVerdict::Accepted => json!({ "result": "accepted" }),
        SinkVerdict::Rejected {
            push_index,
            message,
        } => json!({ "result": "rejected", "push_index": push_index, "message": message }),
    };
    let entry = json!({
        "case": candidate.case,
        "sample": candidate.sample,
        "offset": candidate.offset,
        "input_points_f64": candidate.internal,
        "dir_in": candidate.dir_in,
        "dir_out": candidate.dir_out,
        "quantized_points_bits": record.observation.points.iter().copied().map(bits).collect::<Vec<_>>(),
        "quantized_points": record.observation.points,
        "chords_m": record.observation.chords,
        "turns_deg": record.observation.turns_deg,
        "sink": sink,
        "compiler_edge_interior": verdict_json(&record.edge_interior, &record.observation.points),
        "compiler_full_path": verdict_json(&record.full_path, &record.observation.points),
    });
    println!("S0-JSON {entry}");
    // 分层蕴含：sink 接受 ⇒ 同一组规范点的 compiler 边内硬门接受。
    if record.sink == SinkVerdict::Accepted {
        assert!(
            record.edge_interior.accepted(),
            "sink accepted but compiler edge-interior rejected: {} {}",
            candidate.case,
            candidate.sample
        );
    }
    record
}

fn assert_same_points(verdict: &CompileVerdict, observation: &Observation) {
    let CompileVerdict::Accepted(path) = verdict else {
        panic!("expected accepted compile");
    };
    assert!(
        path.internal
            .points
            .iter()
            .copied()
            .map(bits)
            .eq(observation.points.iter().copied().map(bits)),
        "compiled internal points differ from the candidate canonical points"
    );
}

/// 观测量与锁定值的比较容差：角度 1e-9°、长度 1e-12 m（均远小于判定余量）。
fn assert_close(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: {actual} vs {expected}"
    );
}

/// 原点见证的规范点位模式（t = 0.5 划分后逐分量 `f32` 位模式）。
const ORIGIN_WITNESS_BITS: [[u32; 3]; 3] = [
    [0, 0, 0],
    [1_037_275_609, 0, 987_409_756],
    [1_045_656_992, 0, 1_004_179_506],
];

#[test]
fn s0_a_origin_witness_two_pieces_through_sink() {
    let record = evaluate(&witness_candidate("S0-A", "origin", [0.0; 3]));
    let obs = &record.observation;
    let point_bits = obs.points.iter().copied().map(bits).collect::<Vec<_>>();
    assert_eq!(point_bits, ORIGIN_WITNESS_BITS);
    // 原注释假设：两段均 > 0.1 m 硬下限、< 0.105 m 质量目标，中间 weld ≤ 1.95°。
    for &chord in &obs.chords {
        assert_close(f64::from(chord), 0.103_318_996_727_466_58, 1e-12, "chord");
        assert!(chord > 0.1 && chord < 0.105, "chord {chord}");
    }
    assert_eq!(obs.turns_deg.len(), 1);
    assert_close(obs.turns_deg[0], 1.849_517_821_845_084_3, 1e-9, "turn");
    assert_eq!(record.sink, SinkVerdict::Accepted);
}

#[test]
fn s0_b_origin_witness_real_compile() {
    let record = evaluate(&witness_candidate("S0-B", "origin", [0.0; 3]));
    for verdict in [&record.edge_interior, &record.full_path] {
        assert_same_points(verdict, &record.observation);
        let CompileVerdict::Accepted(path) = verdict else {
            unreachable!("checked by assert_same_points");
        };
        assert_eq!(path.internal.length_mm, 207);
        assert_close(
            f64::from(path.internal.arc_length_meters),
            0.206_637_993_454_933_17,
            1e-12,
            "arc",
        );
        let p = &path.internal.points;
        assert_eq!(
            bits(path.entry.points[path.entry.points.len() - 1]),
            bits(p[0])
        );
        assert_eq!(bits(p[p.len() - 1]), bits(path.exit.points[0]));
        assert_eq!(path.entry.length_mm, 2_000);
        assert_eq!(path.exit.length_mm, 2_000);
    }
    // 完整路径：入/出口沿声明切向时两处跨边转角各约 0.925°，均在 2° 内。
    let CompileVerdict::Accepted(path) = &record.full_path else {
        unreachable!("checked above");
    };
    assert_close(
        turn_between(&path.entry.points, &path.internal.points),
        0.925_241_009_965_587_7,
        1e-9,
        "entry seam",
    );
    assert_close(
        turn_between(&path.internal.points, &path.exit.points),
        0.925_240_988_347_081,
        1e-9,
        "exit seam",
    );
}

/// S0-C 单个偏移样本的锁定结果：三层是否接受及中间量化转角。
struct OffsetExpectation {
    sample: &'static str,
    offset: Vec3,
    accepted: bool,
    turn_deg: f64,
}

#[test]
fn s0_c_coordinate_offsets_layered_verdicts() {
    let corner = |sx: f64, sz: f64| [sx * LUST_HALF_X, 0.0, sz * LUST_HALF_Z];
    let expect = |sample, offset, accepted, turn_deg| OffsetExpectation {
        sample,
        offset,
        accepted,
        turn_deg,
    };
    let expectations = [
        expect("origin", [0.0; 3], true, 1.849_517_821_845_084_3),
        expect(
            "lust-corner(-1,-1)",
            corner(-1.0, -1.0),
            false,
            2.155_653_557_436_747_5,
        ),
        expect(
            "lust-corner(+1,-1)",
            corner(1.0, -1.0),
            false,
            2.173_546_924_331_023,
        ),
        expect(
            "lust-corner(-1,+1)",
            corner(-1.0, 1.0),
            false,
            2.155_653_557_436_747_5,
        ),
        expect(
            "lust-corner(+1,+1)",
            corner(1.0, 1.0),
            false,
            2.173_546_924_331_023,
        ),
        expect(
            "lust-interior(0.5,0.5)",
            corner(0.5, 0.5),
            true,
            1.758_933_747_859_499,
        ),
        expect(
            "lust-interior(-0.5,0.25)",
            corner(-0.5, 0.25),
            true,
            1.826_504_391_997_073_2,
        ),
        expect(
            "lust-interior(0.25,-0.75)",
            corner(0.25, -0.75),
            true,
            1.628_003_037_157_902_2,
        ),
        expect(
            "lust-interior(-0.85,0.2411)",
            corner(-0.85, 0.2411),
            true,
            1.835_304_512_582_801_6,
        ),
        // 域外压力样本：x 超出 LuST 规范坐标域，不代表真实 LuST 数据。
        expect(
            "stress-outside(-6981.1,-4986.5)",
            [-6_981.1, 0.0, -4_986.5],
            false,
            2.173_546_924_331_023,
        ),
    ];
    for expectation in &expectations {
        let record = evaluate(&witness_candidate(
            "S0-C",
            expectation.sample,
            expectation.offset,
        ));
        let obs = &record.observation;
        assert!(obs.chords.iter().all(|&c| c > 0.1 && c < 0.105));
        assert_close(
            obs.turns_deg[0],
            expectation.turn_deg,
            1e-9,
            expectation.sample,
        );
        if expectation.accepted {
            assert_eq!(record.sink, SinkVerdict::Accepted, "{}", expectation.sample);
            assert_same_points(&record.edge_interior, obs);
            assert_same_points(&record.full_path, obs);
        } else {
            // 量化后中间 weld > 2°：sink 与 compiler 边内硬门都拒绝；完整路径在
            // 同一准入阶段先行失败，未到达跨边检查。
            assert!(obs.turns_deg[0] > 2.0, "{}", expectation.sample);
            assert!(
                matches!(
                    &record.sink,
                    SinkVerdict::Rejected { push_index: 1, message }
                        if message.contains("quantized weld budget")
                ),
                "{}",
                expectation.sample
            );
            for verdict in [&record.edge_interior, &record.full_path] {
                assert!(
                    verdict.rejected_with(
                        Stage::ModuleAdmission,
                        "NumericFreeze(DirectionDiscontinuity)"
                    ),
                    "{}: {}",
                    expectation.sample,
                    verdict_json(verdict, &obs.points)
                );
            }
        }
    }
}

#[test]
fn s0_c2_conservative_band_sink_rejects_compiler_accepts() {
    let record = evaluate(&bend_candidate("S0-C2", "origin-1.97deg", 1.0, 1.97));
    let obs = &record.observation;
    // 先确认量化后的实际转角落在 (1.95°, 2°)：sink 保守拒绝、compiler 硬门接受
    // 是预期差异，不是待修复缺陷。
    let turn = obs.turns_deg[0];
    assert!(turn > 1.95 && turn < 2.0, "quantized turn {turn}");
    assert!(obs.chords.iter().all(|&c| c > 0.5));
    assert!(matches!(
        &record.sink,
        SinkVerdict::Rejected { push_index: 1, message } if message.contains("quantized weld budget")
    ));
    assert_same_points(&record.edge_interior, obs);
    assert_same_points(&record.full_path, obs);
    let CompileVerdict::Accepted(path) = &record.full_path else {
        unreachable!("checked by assert_same_points");
    };
    assert_eq!(path.internal.length_mm, 2_000);
}

#[test]
fn s0_d_short_chord_rejected_by_sink_and_compiler() {
    let mut candidate = witness_candidate("S0-D", "origin-scaled-0.96", [0.0; 3]);
    let origin = candidate.internal[0];
    for point in &mut candidate.internal {
        *point = add(origin, scale(sub(*point, origin), 0.96));
    }
    let record = evaluate(&candidate);
    let obs = &record.observation;
    // 两条量化弦非零且 ≤ 0.1 m；中间转角与原见证相同，不会被方向门提前遮蔽。
    assert!(
        obs.chords.iter().all(|&c| c > 0.0 && c <= 0.1),
        "chords {:?}",
        obs.chords
    );
    assert!(obs.turns_deg[0] < 1.95);
    assert!(matches!(
        &record.sink,
        SinkVerdict::Rejected { push_index: 0, message } if message.contains("degenerate canonical segment")
    ));
    // 0.1 m 下限在 HIR 空间冻结执行（道路编辑准入只拒零长弦）。
    for verdict in [&record.edge_interior, &record.full_path] {
        assert!(
            verdict.rejected_with(
                Stage::Compile,
                "lane_edge_key: \"internal\", related_lane_edge_key: None, \
                 violation: DegenerateSegment { segment_index: 0"
            ),
            "{}",
            verdict_json(verdict, &obs.points)
        );
    }
}

#[test]
fn s0_e_over_turn_rejected_by_sink_and_compiler() {
    let record = evaluate(&bend_candidate("S0-E", "origin-2.5deg", 1.0, 2.5));
    let obs = &record.observation;
    assert!(obs.turns_deg[0] > 2.0);
    assert!(obs.chords.iter().all(|&c| c > 0.5));
    assert!(matches!(
        &record.sink,
        SinkVerdict::Rejected { push_index: 1, message } if message.contains("quantized weld budget")
    ));
    // 入/出口边各为单段直线，准入阶段的方向不连续只能来自内部边两段之间。
    for verdict in [&record.edge_interior, &record.full_path] {
        assert!(
            verdict.rejected_with(
                Stage::ModuleAdmission,
                "NumericFreeze(DirectionDiscontinuity)"
            ),
            "{}",
            verdict_json(verdict, &obs.points)
        );
    }
}
