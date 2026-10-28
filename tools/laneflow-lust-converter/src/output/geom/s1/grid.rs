//! #1107：固定参数网格上的量化弦状态搜索；不进入生产采样。

use super::*;

const STEPS: usize = 512;
const STATE_LIMIT: usize = 65_536;
const REPRESENTATIVE_LIMIT: usize = 32;
const FRAME_LIMIT: usize = 256;
const POINT_LIMIT: usize = 262_144;
const DEVIATION_METERS: f64 = 0.01;

mod audit;
mod graph;
mod refine;

pub(super) use graph::deviation as deviation_for_probe;

#[derive(Default, Serialize)]
struct Counts {
    node_evaluations: u64,
    chord_checks: u64,
    transition_checks: u64,
    prefix_point_uses: u64,
    graph_searches: u64,
    states_peak: usize,
    chords_peak: usize,
    frames_peak: usize,
    retained_points_peak: usize,
    state_limited_searches: u64,
    representative_limited_searches: u64,
    duplicate_programs: u64,
    complete_edge_end_rejected: u64,
    edge_replays: u64,
    backtracks: u64,
    start_clamp_rejected: u64,
    previously_reached_chords_skipped: u64,
    refinement_calls: u64,
    refinement_states_peak: usize,
    refinement_chords_peak: usize,
    refinement_skeleton_limited: u64,
}

struct GridFrame {
    control: [[u64; 3]; 6],
    incoming_dir: Option<[u64; 3]>,
    paths: Vec<Vec<usize>>,
    next: usize,
    selected: Option<SavedPrefix>,
}

#[derive(Default)]
struct GridSearch {
    counts: Counts,
    frames: Vec<GridFrame>,
    keys: ExactSeen,
    encountered: usize,
    failed_at: Option<usize>,
    stop: Option<&'static str>,
    spans: Vec<Value>,
    failures: BTreeMap<usize, Value>,
    audit: Option<audit::EndAudit>,
    start_clamp: Option<Vec3>,
    guide: Option<Guide>,
}

#[derive(Clone)]
pub(crate) struct Guide {
    pub(crate) control: [[u64; 3]; 6],
    pub(crate) tail: [f64; 3],
}

impl Guide {
    fn parameters(&self) -> Vec<f64> {
        assert!(
            self.tail[0] > 0.0
                && self.tail[0] < self.tail[1]
                && self.tail[1] < 1.0
                && self.tail[2] == 1.0
        );
        (0..=STEPS)
            .map(|i| i as f64 / STEPS as f64)
            .take_while(|&t| t < self.tail[0])
            .chain(self.tail)
            .collect()
    }
}

impl GridSearch {
    fn work(&self) -> u64 {
        self.counts.node_evaluations
            + self.counts.chord_checks
            + self.counts.transition_checks
            + self.counts.prefix_point_uses
    }

    fn points(&self) -> usize {
        self.keys.points
            + self
                .frames
                .iter()
                .map(|f| {
                    f.paths.iter().map(Vec::len).sum::<usize>()
                        + f.selected.as_ref().map_or(0, |s| s.points.len())
                })
                .sum::<usize>()
    }

    fn fail(&mut self, reason: &'static str) -> Error {
        self.stop = Some(reason);
        Error::SumoModel(format!("S1 网格研究预算停止：{reason}"))
    }

    fn reserve_work(&mut self, amount: u64) -> Result<()> {
        if self.work() + amount > EVALUATION_LIMIT {
            Err(self.fail("work"))
        } else {
            Ok(())
        }
    }

    fn reserve_points(&mut self, amount: usize) -> Result<()> {
        if self.points() + amount > POINT_LIMIT {
            return Err(self.fail("retained_points"));
        }
        self.counts.retained_points_peak =
            self.counts.retained_points_peak.max(self.points() + amount);
        Ok(())
    }
}

thread_local! {
    static GRID: RefCell<Option<GridSearch>> = const { RefCell::new(None) };
}

struct GridReset;
impl Drop for GridReset {
    fn drop(&mut self) {
        GRID.with(|s| *s.borrow_mut() = None);
    }
}

pub(crate) fn policy() -> Value {
    json!({"grid_steps":STEPS,"states_per_cubic":STATE_LIMIT,
        "representatives_per_cubic":REPRESENTATIVE_LIMIT,"frames":FRAME_LIMIT,
        "retained_points":POINT_LIMIT,"work":EVALUATION_LIMIT,
        "programs_per_edge":PROGRAM_LIMIT,"deviation_estimate_m":DEVIATION_METERS,
        "end_degrees":END_DEGREES,"metrics_excluded_from_search_work":true,
        "state_merge":"first BFS prefix per directed chord",
        "initial_chord":"original complete-edge start clamp checked before prefix selection",
        "transition_reuse":"remove directed chord after first BFS reach; preserve remaining order",
        "scope":"retained finite representatives only; not continuous completeness or allocator peak"})
}

pub(crate) fn search(input: &EdgeInput) -> EdgeSearch {
    search_impl(input, false, None)
}

pub(crate) fn search_audited(input: &EdgeInput) -> EdgeSearch {
    search_impl(input, true, None)
}

pub(crate) fn search_guided(input: &EdgeInput, guide: Guide) -> EdgeSearch {
    search_impl(input, true, Some(guide))
}

pub(crate) fn local_probe(control: [Vec3; 6], start_clamp: Option<Vec3>) -> Value {
    audit::probe(control, start_clamp)
}

fn search_impl(input: &EdgeInput, audited: bool, guide: Option<Guide>) -> EdgeSearch {
    let _reset = GridReset;
    let mut state = GridSearch {
        audit: audited.then(audit::EndAudit::default),
        start_clamp: input.clamps[0],
        guide,
        ..GridSearch::default()
    };
    let mut candidates = Vec::new();
    let mut last_error = None;
    let mut exhausted = false;
    loop {
        state.encountered = 0;
        state.failed_at = None;
        state.spans.clear();
        state.counts.edge_replays += 1;
        GRID.with(|slot| {
            assert!(slot.borrow().is_none(), "nested grid search");
            *slot.borrow_mut() = Some(state);
        });
        let repaired = repair_curve(
            &input.points,
            input.clamps[0],
            input.clamps[1],
            input.clamp_sources[0].as_ref(),
            input.clamp_sources[1].as_ref(),
        );
        state = GRID.with(|slot| slot.borrow_mut().take().unwrap());
        let complete = repaired.is_ok();
        match repaired {
            Ok(repaired) => {
                let points = program_points(&repaired.program);
                let dirs = [
                    unit(sub(points[1], points[0])).unwrap(),
                    unit(sub(points[points.len() - 1], points[points.len() - 2])).unwrap(),
                ];
                if let Some(audit) = &mut state.audit {
                    audit.record(input.clamps, dirs, &state.spans);
                }
                if (0..2).any(|i| {
                    input.clamps[i]
                        .is_some_and(|c| angle_rad(dirs[i], c).to_degrees() > END_DEGREES)
                }) {
                    state.counts.complete_edge_end_rejected += 1;
                    last_error = Some("complete edge end constraint".to_owned());
                } else {
                    let key = canonical_key(points.into_iter());
                    if state.keys.keys.contains(&key) {
                        state.counts.duplicate_programs += 1;
                    } else {
                        if state.reserve_points(key.len()).is_err() {
                            break;
                        }
                        assert_eq!(state.keys.insert(key, POINT_LIMIT), Retain::New);
                        candidates.push(Candidate {
                            repaired,
                            spans: state.spans.clone(),
                        });
                        if candidates.len() == PROGRAM_LIMIT {
                            state.stop = Some("program_pool");
                            break;
                        }
                    }
                }
            }
            Err(e) => last_error = Some(e.to_string()),
        }
        if state.stop.is_some() {
            break;
        }
        let advance = state.failed_at.unwrap_or(state.encountered);
        if advance == 0 {
            exhausted = complete || state.failed_at == Some(0);
            break;
        }
        state.counts.backtracks += u64::from(state.failed_at.is_some());
        state.frames.truncate(advance);
        state.frames[advance - 1].selected = None;
    }
    let mut report = json!({"candidate_programs":candidates.len(),"counts":state.counts,
            "search_work":state.work(),"last_error":last_error,"failures":state.failures,
            "retained_family_exhausted":exhausted,"complete_grid_path_enumeration":false,
            "budget_stopped":state.stop.is_some() || state.counts.state_limited_searches > 0,
            "stop_reason":state.stop.unwrap_or(if exhausted {"retained_family_exhausted"} else {"preparation_or_straight_rejected"})});
    if let Some(audit) = &state.audit {
        report["end_audit"] = json!(audit);
    }
    EdgeSearch { report, candidates }
}

pub(super) fn sample_if_active(
    span_index: usize,
    control: [Vec3; 6],
    sink: &mut EmissionSink,
) -> Option<Result<()>> {
    GRID.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|s| sample(s, span_index, control, sink))
    })
}

fn sample(
    state: &mut GridSearch,
    span_index: usize,
    control: [Vec3; 6],
    sink: &mut EmissionSink,
) -> Result<()> {
    let index = state.encountered;
    state.encountered += 1;
    let checkpoint = sink.checkpoint();
    assert_eq!(
        checkpoint.prev_point,
        quantize_vec3(control[0]),
        "fixed grid seam"
    );
    if index == state.frames.len() {
        if index == FRAME_LIMIT {
            return Err(state.fail("frames"));
        }
        let params = state
            .guide
            .as_ref()
            .filter(|g| g.control == control.map(|p| p.map(f64::to_bits)))
            .map(Guide::parameters);
        let result = if let Some(params) = &params {
            graph::paths_on(state, control, checkpoint.prev_dir, params)
        } else {
            graph::paths(state, control, checkpoint.prev_dir)
        };
        let paths = match result {
            Ok(paths) => paths,
            Err(error) => {
                state.failures.insert(
                    index,
                    json!({"span_index_zero_based":span_index,
                    "control_f64_bits":control.map(|p|p.map(f64::to_bits)),
                    "incoming_dir_f64_bits":checkpoint.prev_dir.map(|p|p.map(f64::to_bits)),
                    "reason":state.stop}),
                );
                return Err(error);
            }
        };
        state.reserve_points(paths.iter().map(Vec::len).sum())?;
        if paths.is_empty() {
            state.failures.insert(
                index,
                json!({"span_index_zero_based":span_index,
                "control_f64_bits":control.map(|p|p.map(f64::to_bits)),
                "incoming_dir_f64_bits":checkpoint.prev_dir.map(|p|p.map(f64::to_bits)),
                "reason":"no_retained_grid_path"}),
            );
            if state.guide.is_some() {
                state.failures.get_mut(&index).unwrap()["accepted_prefix_spans"] =
                    json!(state.spans);
            }
        }
        state.frames.push(GridFrame {
            control: control.map(|p| p.map(f64::to_bits)),
            incoming_dir: checkpoint.prev_dir.map(|p| p.map(f64::to_bits)),
            paths,
            next: 0,
            selected: None,
        });
        state.counts.frames_peak = state.counts.frames_peak.max(state.frames.len());
    }
    assert_eq!(
        state.frames[index].control,
        control.map(|p| p.map(f64::to_bits)),
        "fixed grid control"
    );
    assert_eq!(
        state.frames[index].incoming_dir,
        checkpoint.prev_dir.map(|p| p.map(f64::to_bits)),
        "fixed grid incoming direction"
    );
    if state.frames[index].selected.is_none() {
        let guide = state
            .guide
            .as_ref()
            .filter(|g| g.control == control.map(|p| p.map(f64::to_bits)));
        let parameter_grid = guide.map(Guide::parameters);
        let frame = &mut state.frames[index];
        let Some(path) = frame.paths.get(frame.next) else {
            state.failed_at = Some(index);
            return Err(Error::SumoModel(format!("S1 网格代表耗尽：cubic {index}")));
        };
        let rank = frame.next;
        let params = path
            .iter()
            .map(|&k| {
                parameter_grid
                    .as_ref()
                    .map_or(k as f64 / STEPS as f64, |p| p[k])
            })
            .collect::<Vec<_>>();
        let points = params
            .iter()
            .skip(1)
            .map(|&t| {
                if t == 1.0 {
                    control[3]
                } else {
                    cubic_at(control[0], control[1], control[2], control[3], t)
                }
            })
            .collect::<Vec<_>>();
        state.reserve_points(points.len())?;
        state.reserve_work(points.len() as u64)?;
        state.counts.node_evaluations += points.len() as u64;
        let report = json!({"cubic_index":index,"span_index_zero_based":span_index,
            "partition":if parameter_grid.is_some(){"grid_prefix_with_witness_tail"}else{"grid_512_directed_chord_bfs"},"candidate_rank":rank,
            "parameter_indices":state.frames[index].paths[rank],
            "control_f64_bits":control[..4].iter().map(|p|p.map(f64::to_bits)).collect::<Vec<_>>(),
            "metrics":metrics([control[0],control[1],control[2],control[3]],&params)});
        let report = if parameter_grid.is_some() {
            let mut report = report;
            report["parameters_f64_bits"] =
                json!(params.iter().map(|t| t.to_bits()).collect::<Vec<_>>());
            report
        } else {
            report
        };
        state.frames[index].next += 1;
        state.frames[index].selected = Some(SavedPrefix { points, report });
    }
    let count = state.frames[index].selected.as_ref().unwrap().points.len();
    state.reserve_work(count as u64)?;
    let saved = state.frames[index].selected.as_ref().unwrap();
    for &point in &saved.points {
        state.counts.prefix_point_uses += 1;
        if let Err(error) = sink.push(point) {
            sink.restore(&checkpoint);
            return Err(error);
        }
    }
    state.spans.push(saved.report.clone());
    Ok(())
}

#[test]
fn work_and_retained_point_limits_stop_before_sink_mutation() {
    let control = [
        [0.0, 0.0, 0.0],
        [3.0, 0.1, 0.0],
        [7.0, 0.1, 0.0],
        [10.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let mut state = GridSearch::default();
    state.counts.node_evaluations = EVALUATION_LIMIT;
    let mut sink = EmissionSink::new(control[0]);
    assert!(sample(&mut state, 0, control, &mut sink).is_err());
    assert_eq!(state.stop, Some("work"));
    assert_eq!(state.work(), EVALUATION_LIMIT);
    assert!(sink.segments.is_empty());
    let mut state = GridSearch::default();
    state.keys.points = POINT_LIMIT;
    assert!(sample(&mut state, 0, control, &mut sink).is_err());
    assert_eq!(state.stop, Some("retained_points"));
    assert!(sink.segments.is_empty());
}

#[test]
fn exhausted_suffix_advances_prefix_and_replay_keeps_the_sink_contract() {
    let control = [
        [0.0, 0.0, 0.0],
        [3.0, 0.3, 0.0],
        [7.0, 0.3, 0.0],
        [10.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let mut state = GridSearch::default();
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    let first = sink.checkpoint();
    let searches = state.counts.graph_searches;
    state.encountered = 0;
    let mut again = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut again).unwrap();
    assert_eq!(first.prev_point, again.prev_point);
    assert_eq!(first.prev_dir, again.prev_dir);
    assert_eq!(
        first.total_arc_meters.to_bits(),
        again.total_arc_meters.to_bits()
    );
    assert_eq!(searches, state.counts.graph_searches);
    let mut impossible = control;
    impossible[0] = control[3];
    impossible[1] = control[3];
    impossible[2] = add(control[3], [0.05, 0.0, 0.0]);
    impossible[3] = impossible[2];
    assert!(sample(&mut state, 1, impossible, &mut again).is_err());
    assert_eq!(state.failed_at, Some(1));
    assert_eq!(first.prev_point, again.prev_point);
    state.frames.truncate(1);
    state.frames[0].selected = None;
    state.encountered = 0;
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    assert_eq!(state.frames[0].next, 2);
}
