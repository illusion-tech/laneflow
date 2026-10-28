//! #1093：固定 cubic 的扫描复用、精确去重与有界前缀回溯；仅用于测试研究。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::output::emit::observe::EdgeInput;

pub(crate) const PROGRAM_LIMIT: usize = 8;
pub(crate) const PARTITION_LIMIT: u64 = 32_768;
pub(crate) const EVALUATION_LIMIT: u64 = 32_000_000;
const END_DEGREES: f64 = 0.95;
const CACHED_CUBIC_LIMIT: usize = 256;
const SCAN_STEPS: usize = 512;
const RETAINED_KEY_POINT_LIMIT: usize = 262_144;
const RETAINED_PREFIX_POINT_LIMIT: usize = 262_144;

#[derive(Clone, Copy)]
pub(crate) struct SearchBudget {
    pub(crate) partitions: u64,
}

impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            partitions: PARTITION_LIMIT,
        }
    }
}

struct SavedPrefix {
    points: Vec<Vec3>,
    report: Value,
}

struct Frame {
    control: [[u64; 3]; 6],
    incoming_point: [u32; 3],
    incoming_dir: Option<[u64; 3]>,
    next_partition: u32,
    rank: usize,
    seen: ExactSeen,
    selected: Option<SavedPrefix>,
}

#[derive(Default)]
struct ExactSeen {
    keys: BTreeSet<Vec<[u32; 3]>>,
    points: usize,
}

#[derive(Debug, PartialEq)]
enum Retain {
    New,
    Duplicate,
    Budget,
}

impl ExactSeen {
    fn insert(&mut self, key: Vec<[u32; 3]>, limit: usize) -> Retain {
        if self.keys.contains(&key) {
            return Retain::Duplicate;
        }
        if self.points + key.len() > limit {
            return Retain::Budget;
        }
        self.points += key.len();
        self.keys.insert(key);
        Retain::New
    }
}

fn canonical_key(points: impl Iterator<Item = Vec3>) -> Vec<[u32; 3]> {
    points.map(|p| quantize_vec3(p).map(f32::to_bits)).collect()
}

struct Scan {
    control: [[u64; 3]; 6],
    cumulative: Option<Vec<f64>>,
}

#[derive(Clone, Default, Serialize)]
struct Counts {
    partitions: u64,
    point_evaluations: u64,
    tangent_evaluations: u64,
    sink_rejected: u64,
    unavailable_partition: u64,
    complete_edge_end_rejected: u64,
    edge_replays: u64,
    scan_builds: u64,
    scan_reuses: u64,
    cached_cubics_peak: usize,
    scan_values_peak: usize,
    duplicate_partitions: u64,
    duplicate_programs: u64,
    retained_key_points_peak: usize,
    retained_prefix_points_peak: usize,
    prefix_reuses: u64,
    prefix_point_uses: u64,
    cubic_sink_pushes: u64,
    backtracks: u64,
    frames_peak: usize,
}

#[derive(Default)]
struct Search {
    budget: SearchBudget,
    frames: Vec<Frame>,
    encountered: usize,
    failed_at: Option<usize>,
    truncated: bool,
    counts: Counts,
    spans: Vec<Value>,
    scans: BTreeMap<usize, Scan>,
    program_keys: ExactSeen,
    truncation_reason: Option<&'static str>,
}

impl Search {
    fn key_points(&self) -> usize {
        self.program_keys.points + self.frames.iter().map(|f| f.seen.points).sum::<usize>()
    }

    fn prefix_points(&self) -> usize {
        self.frames
            .iter()
            .filter_map(|f| f.selected.as_ref())
            .map(|p| p.points.len())
            .sum()
    }

    fn work(&self) -> u64 {
        self.counts.point_evaluations
            + self.counts.tangent_evaluations
            + self.counts.prefix_point_uses
    }

    fn truncate(&mut self, reason: &'static str) -> Error {
        self.truncated = true;
        self.truncation_reason = Some(reason);
        Error::SumoModel(format!("S1 研究预算截断：{reason}"))
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Search>> = const { RefCell::new(None) };
}

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        ACTIVE.with(|slot| *slot.borrow_mut() = None);
    }
}

pub(crate) struct Candidate {
    pub(crate) repaired: RepairedCurve,
    pub(crate) spans: Vec<Value>,
}

pub(crate) struct EdgeSearch {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) report: Value,
}

pub(crate) fn policy(budget: SearchBudget) -> Value {
    json!({"pieces_max":SAMPLE_MAX_PIECES,"partitions_per_edge":budget.partitions,
        "evaluations_per_edge":EVALUATION_LIMIT,"programs_per_edge":PROGRAM_LIMIT,
        "end_degrees":END_DEGREES,"recursive_candidate_search":false,
        "cached_cubics_and_frames":CACHED_CUBIC_LIMIT,"scan_steps":SCAN_STEPS,
        "retained_canonical_key_points":RETAINED_KEY_POINT_LIMIT,
        "retained_prefix_points":RETAINED_PREFIX_POINT_LIMIT,
        "evaluation_accounting":"new candidate point uses + tangent evaluations + replayed prefix point uses",
        "dedup":"complete ordered canonical f32 point bits; first representative",
        "traversal":"depth-first persistent cursors; discard descendants when parent changes",
        "scope":"test research only; storage counts are not allocator peak measurements"})
}

pub(crate) fn search(input: &EdgeInput, budget: SearchBudget) -> EdgeSearch {
    assert!(budget.partitions > 0, "nonzero partition budget");
    let mut state = Search {
        budget,
        ..Search::default()
    };
    let mut candidates = Vec::new();
    let mut last_error = None;
    let mut family_exhausted = false;
    let _reset = Reset;
    loop {
        state.encountered = 0;
        state.failed_at = None;
        state.spans.clear();
        state.counts.edge_replays += 1;
        ACTIVE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested S1 search");
            *slot.borrow_mut() = Some(state);
        });
        let repaired = repair_curve(
            &input.points,
            input.clamps[0],
            input.clamps[1],
            input.clamp_sources[0].as_ref(),
            input.clamp_sources[1].as_ref(),
        );
        state = ACTIVE.with(|slot| slot.borrow_mut().take().expect("active search"));
        let complete = repaired.is_ok();
        match repaired {
            Ok(repaired) => {
                let points = program_points(&repaired.program);
                let dirs = [
                    unit(sub(points[1], points[0])).expect("first chord"),
                    unit(sub(points[points.len() - 1], points[points.len() - 2]))
                        .expect("last chord"),
                ];
                let angles =
                    [0, 1].map(|i| input.clamps[i].map(|c| angle_rad(dirs[i], c).to_degrees()));
                if angles.iter().flatten().any(|&a| a > END_DEGREES) {
                    state.counts.complete_edge_end_rejected += 1;
                    last_error = Some(format!("complete edge end constraint: {angles:?}"));
                } else {
                    match state.program_keys.insert(
                        canonical_key(points.into_iter()),
                        RETAINED_KEY_POINT_LIMIT
                            - state.frames.iter().map(|f| f.seen.points).sum::<usize>(),
                    ) {
                        Retain::Duplicate => {
                            state.counts.duplicate_programs += 1;
                        }
                        Retain::Budget => {
                            state.truncate("retained_keys");
                            break;
                        }
                        Retain::New => {
                            state.counts.retained_key_points_peak = state
                                .counts
                                .retained_key_points_peak
                                .max(state.key_points());
                            candidates.push(Candidate {
                                repaired,
                                spans: state.spans.clone(),
                            });
                            if candidates.len() == PROGRAM_LIMIT {
                                state.truncate("program_pool");
                                break;
                            }
                        }
                    }
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }
        if state.truncated {
            break;
        }
        // 仅改变当前最深层游标；父层保留规范前缀，改变父层后丢弃其后全部帧。
        let advance = state.failed_at.unwrap_or(state.encountered);
        if advance == 0 {
            family_exhausted = state.failed_at == Some(0) || complete;
            break;
        }
        if state.failed_at.is_some() {
            state.counts.backtracks += 1;
        }
        state.frames.truncate(advance);
        state.frames[advance - 1].selected = None;
    }
    EdgeSearch {
        report: json!({"candidate_programs": candidates.len(), "family_exhausted": family_exhausted,
            "budget_truncated": state.truncated, "stop_reason": state.truncation_reason.unwrap_or(if family_exhausted {"finite_family_exhausted"} else {"preparation_or_straight_rejected"}),
            "counts": state.counts, "last_error": last_error}),
        candidates,
    }
}

pub(crate) mod feasibility;
pub(crate) mod grid;
pub(crate) mod reshape;
mod traverse;

pub(super) fn sample_if_active(
    span_index: usize,
    control: [Vec3; 6],
    sink: &mut EmissionSink,
) -> Option<Result<()>> {
    if reshape::capture_if_active(control) {
        return Some(Ok(()));
    }
    if let Some(result) = grid::sample_if_active(span_index, control, sink) {
        return Some(result);
    }
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|state| traverse::sample(state, span_index, control, sink))
    })
}

#[test]
fn dedup_uses_all_canonical_points_and_respects_budget() {
    let mut seen = ExactSeen::default();
    let a = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let mut same = a;
    same[1][0] += 1e-9;
    let mut changed = a;
    changed[1][1] = 0.01;
    assert_eq!(seen.insert(canonical_key(a.into_iter()), 6), Retain::New);
    assert_eq!(
        seen.insert(canonical_key(same.into_iter()), 6),
        Retain::Duplicate
    );
    assert_eq!(seen.points, 3);
    assert_eq!(
        seen.insert(canonical_key(changed.into_iter()), 6),
        Retain::New
    );
    assert_eq!(
        seen.insert(canonical_key([[3.0, 0.0, 0.0]].into_iter()), 6),
        Retain::Budget
    );
    assert_eq!(seen.points, 6);
}

fn scan(span: &BezierSpan) -> (Option<Vec<f64>>, u64) {
    let Some(mut prev) = unit(cubic_tangent_at(span.a, span.c1, span.c2, span.b, 0.0)) else {
        return (None, 1);
    };
    let mut cumulative = vec![0.0; SCAN_STEPS + 1];
    for k in 1..=SCAN_STEPS {
        let dir = unit(cubic_tangent_at(
            span.a,
            span.c1,
            span.c2,
            span.b,
            k as f64 / SCAN_STEPS as f64,
        ))
        .unwrap_or(prev);
        cumulative[k] = cumulative[k - 1] + angle_rad(prev, dir);
        prev = dir;
    }
    if cumulative[SCAN_STEPS] <= 1e-12 {
        (None, 513)
    } else {
        (Some(cumulative), 513)
    }
}

fn split_params(cumulative: &[f64], n: u32) -> Vec<f64> {
    let mut params = vec![0.0; n as usize + 1];
    *params.last_mut().unwrap() = 1.0;
    let mut scan = 0;
    for (k, slot) in params.iter_mut().enumerate().take(n as usize).skip(1) {
        let target = cumulative[SCAN_STEPS] * k as f64 / n as f64;
        while scan + 1 < cumulative.len() && cumulative[scan + 1] < target {
            scan += 1;
        }
        let t = if cumulative[scan + 1] > cumulative[scan] {
            let f = (target - cumulative[scan]) / (cumulative[scan + 1] - cumulative[scan]);
            (scan as f64 + f) / SCAN_STEPS as f64
        } else {
            scan as f64 / SCAN_STEPS as f64
        };
        *slot = t.clamp(0.0, 1.0);
    }
    params
}

#[test]
fn reused_scan_matches_original_parameter_bits() {
    for c in [
        [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        [
            [7_300.0, 0.0, 0.0],
            [7_300.2, 0.3, 0.0],
            [7_301.0, 0.8, 0.0],
            [7_301.1, 0.0, 0.0],
        ],
        [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
    ] {
        let span = BezierSpan {
            a: c[0],
            c1: c[1],
            c2: c[2],
            b: c[3],
            dir_a: [1.0, 0.0, 0.0],
            dir_b: [1.0, 0.0, 0.0],
        };
        let (table, _) = scan(&span);
        for n in [2, 7, 32, 257, 2_048] {
            let actual = table.as_deref().map(|t| {
                split_params(t, n)
                    .into_iter()
                    .map(f64::to_bits)
                    .collect::<Vec<_>>()
            });
            let expected = tangent_angle_split_params(&span, n)
                .map(|v| v.into_iter().map(f64::to_bits).collect::<Vec<_>>());
            assert_eq!(actual, expected);
        }
    }
}

pub(crate) fn program_points(program: &re::RoadEditingCurveProgram) -> Vec<Vec3> {
    let point = |p: re::RoadEditingPoint3| quantize_vec3([p.x(), p.y(), p.z()]).map(f64::from);
    std::iter::once(point(program.start()))
        .chain(program.segments().iter().map(|segment| {
            let re::RoadEditingCurveSegmentGeometry::Line { end } = segment.geometry() else {
                panic!("line program");
            };
            point(end)
        }))
        .collect()
}

pub(crate) fn program_bits(program: &re::RoadEditingCurveProgram) -> Value {
    let point = |p: re::RoadEditingPoint3| [p.x(), p.y(), p.z()].map(f64::to_bits);
    json!({"start": point(program.start()), "segments": program.segments().iter().map(|s| {
        let re::RoadEditingCurveSegmentGeometry::Line {end} = s.geometry() else {panic!("line program");};
        json!({"end":point(end), "canvas_selection":s.canvas_selection()})
    }).collect::<Vec<_>>()})
}

fn split(c: [Vec3; 4], t: f64) -> ([Vec3; 4], [Vec3; 4]) {
    let lerp = |p, q| add(p, scale(sub(q, p), t));
    let p01 = lerp(c[0], c[1]);
    let p12 = lerp(c[1], c[2]);
    let p23 = lerp(c[2], c[3]);
    let p012 = lerp(p01, p12);
    let p123 = lerp(p12, p23);
    let p = lerp(p012, p123);
    ([c[0], p01, p012, p], [p, p123, p23, c[3]])
}

fn restricted(c: [Vec3; 4], a: f64, b: f64) -> [Vec3; 4] {
    let left = if b == 1.0 { c } else { split(c, b).0 };
    if a == 0.0 { left } else { split(left, a / b).1 }
}

fn distance_to_chord(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let d = sub(b, a);
    let t = (dot(sub(p, a), d) / dot(d, d)).clamp(0.0, 1.0);
    norm(sub(p, add(a, scale(d, t))))
}

fn metrics(c: [Vec3; 4], params: &[f64]) -> Value {
    let eval = |t| {
        if t == 0.0 {
            c[0]
        } else if t == 1.0 {
            c[3]
        } else {
            cubic_at(c[0], c[1], c[2], c[3], t)
        }
    };
    let mut bound = 0.0_f64;
    let mut observed = 0.0_f64;
    let mut length = 0.0;
    for w in params.windows(2) {
        let subcurve = restricted(c, w[0], w[1]);
        let a = quantize_vec3(eval(w[0])).map(f64::from);
        let b = quantize_vec3(eval(w[1])).map(f64::from);
        let line = [
            a,
            add(a, scale(sub(b, a), 1.0 / 3.0)),
            add(a, scale(sub(b, a), 2.0 / 3.0)),
            b,
        ];
        for i in 0..4 {
            bound = bound.max(norm(sub(subcurve[i], line[i])));
        }
        for k in 0..=8 {
            observed = observed.max(distance_to_chord(
                eval(w[0] + (w[1] - w[0]) * f64::from(k) / 8.0),
                a,
                b,
            ));
        }
        length += f64::from(quantized_chord_meters(
            a.map(|v| v as f32),
            b.map(|v| v as f32),
        ));
    }
    // 弦下界 / 控制多边形上界只在实数算术下成立；此处普通 f64 估计不作认证。
    let mut lower = 0.0;
    let mut upper = 0.0;
    for k in 0..4_096 {
        let p = restricted(c, f64::from(k) / 4_096.0, f64::from(k + 1) / 4_096.0);
        lower += norm(sub(p[3], p[0]));
        upper += p.windows(2).map(|w| norm(sub(w[1], w[0]))).sum::<f64>();
    }
    json!({"deviation_control_hull_estimate_m":bound, "deviation_grid_to_own_chord_m":observed,
        "grid_points_per_interval":9, "quantized_arc_m":length,
        "cubic_length_chord_estimate_m":lower, "cubic_length_polygon_estimate_m":upper,
        "length_intervals":4_096, "certified_continuous_bound":false})
}

#[test]
fn metrics_cover_interior_not_just_vertices() {
    let c = [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let m = metrics(c, &[0.0, 1.0]);
    assert_eq!(m["deviation_grid_to_own_chord_m"], 0.75);
    assert!(m["deviation_control_hull_estimate_m"].as_f64().unwrap() >= 0.75);
    assert!((m["cubic_length_chord_estimate_m"].as_f64().unwrap() - 2.0).abs() < 0.000_001);
}

#[test]
fn finite_search_restores_sink_on_failure() {
    let a = [0.0, 0.0, 0.0];
    let b = [0.05, 0.0, 0.0];
    let mut sink = EmissionSink::new(a);
    let before = sink.checkpoint();
    ACTIVE.with(|s| *s.borrow_mut() = Some(Search::default()));
    let _reset = Reset;
    assert!(
        sample_if_active(0, [a, a, b, b, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &mut sink)
            .unwrap()
            .is_err()
    );
    assert_eq!(sink.segments.len(), before.segments_len);
    assert_eq!(sink.prev_point, before.prev_point);
    assert_eq!(sink.total_arc_meters, before.total_arc_meters);
    ACTIVE.with(|s| {
        let s = s.borrow();
        let s = s.as_ref().unwrap();
        assert_eq!(s.counts.partitions, 4_094);
        assert!(!s.truncated);
    });
}

#[test]
fn work_limit_is_truncation_before_next_partition() {
    let mut state = Search::default();
    state.counts.partitions = PARTITION_LIMIT;
    ACTIVE.with(|s| *s.borrow_mut() = Some(state));
    let _reset = Reset;
    let a = [0.0, 0.0, 0.0];
    let b = [1.0, 0.0, 0.0];
    let mut sink = EmissionSink::new(a);
    assert!(
        sample_if_active(0, [a, a, b, b, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &mut sink)
            .unwrap()
            .is_err()
    );
    ACTIVE.with(|s| assert!(s.borrow().as_ref().unwrap().truncated));
    assert!(sink.segments.is_empty());
}
