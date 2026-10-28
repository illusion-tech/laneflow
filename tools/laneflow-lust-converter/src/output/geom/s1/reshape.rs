//! #1115：失败边内部几何的有限研究族；来源拟合与完整控制器验收分开。

use super::*;

mod quality;
mod tests;

type Control = [Vec3; 6];
const WORK_LIMIT: u64 = 32_000_000;
const FIT_KEY_POINT_LIMIT: usize = 262_144;
const FACTORS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0];
const MAX_PIECES: u32 = 96;
const MAX_CURVES: usize = 6;

thread_local! {
    static REFERENCE: RefCell<Option<Vec<Control>>> = const { RefCell::new(None) };
}

struct CaptureReset;

impl Drop for CaptureReset {
    fn drop(&mut self) {
        REFERENCE.with(|slot| *slot.borrow_mut() = None);
    }
}

pub(super) fn capture_if_active(control: Control) -> bool {
    REFERENCE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(curves) = slot.as_mut() else {
            return false;
        };
        assert!(curves.len() < MAX_CURVES, "bounded reference capture");
        curves.push(control);
        true
    })
}

pub(crate) fn capture_line_if_active(a: Vec3, b: Vec3, direction: Vec3) -> bool {
    capture_if_active([
        a,
        add(a, scale(sub(b, a), 1.0 / 3.0)),
        add(a, scale(sub(b, a), 2.0 / 3.0)),
        b,
        direction,
        direction,
    ])
}

fn reference(input: &EdgeInput) -> Vec<Control> {
    REFERENCE.with(|slot| {
        assert!(slot.borrow().is_none(), "nested reference capture");
        *slot.borrow_mut() = Some(Vec::new());
    });
    let _reset = CaptureReset;
    // 观察模式不向 sink 写入点，末尾空程序不作为研究候选返回。
    let _discarded = repair_curve(
        &input.points,
        input.clamps[0],
        input.clamps[1],
        input.clamp_sources[0].as_ref(),
        input.clamp_sources[1].as_ref(),
    );
    let curves = REFERENCE.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(!curves.is_empty(), "cubic reference required");
    assert_eq!(curves[0][0], input.points[0], "reference starts at source");
    assert_eq!(
        curves.last().unwrap()[3],
        *input.points.last().unwrap(),
        "complete reference end"
    );
    assert!(
        curves.windows(2).all(|w| w[0][3] == w[1][0]),
        "complete reference chain"
    );
    curves
}

#[derive(Default, Serialize)]
struct Budget {
    partitions: u64,
    sample_work: u64,
    quality_work: u64,
    retained_key_points: usize,
    stop: Option<&'static str>,
}

impl Budget {
    fn sample(&mut self, n: u64) -> bool {
        if n > WORK_LIMIT - self.sample_work {
            self.stop = Some("sampling_work");
            false
        } else {
            self.sample_work += n;
            true
        }
    }

    fn quality(&mut self, n: u64) -> bool {
        if n > WORK_LIMIT - self.quality_work {
            self.stop = Some("source_fit_work");
            false
        } else {
            self.quality_work += n;
            true
        }
    }

    fn partition(&mut self) -> bool {
        if self.partitions == PARTITION_LIMIT {
            self.stop = Some("partitions");
            false
        } else {
            self.partitions += 1;
            true
        }
    }
}

struct Geometry {
    curves: Vec<Control>,
    description: Value,
}

fn geometries(original: &[Control]) -> Vec<Geometry> {
    let mut result = vec![Geometry {
        curves: original.to_vec(),
        description: json!({"group":"reference"}),
    }];
    for (label, group_size) in [("keep", 1), ("pair", 2), ("whole", original.len())] {
        for start in FACTORS {
            for finish in FACTORS {
                let curves = original
                    .chunks(group_size)
                    .map(|group| {
                        let first = group[0];
                        let last = group.last().unwrap();
                        let scale = norm(sub(last[3], first[0])) / 3.0;
                        [
                            first[0],
                            add(first[0], super::scale(first[4], scale * start)),
                            sub(last[3], super::scale(last[5], scale * finish)),
                            last[3],
                            first[4],
                            last[5],
                        ]
                    })
                    .collect();
                result.push(Geometry {
                    curves,
                    description: json!({"group":label,"handle_factors":[start,finish]}),
                });
            }
        }
    }
    assert_eq!(result.len(), 148);
    result
}

fn monotone(curves: &[Control]) -> bool {
    curves.iter().all(|c| {
        let chord = sub(c[3], c[0]);
        c[..4]
            .windows(2)
            .all(|w| dot(sub(w[1], w[0]), chord) >= 0.0)
    })
}

fn bits(curves: &[Control]) -> Vec<[[u64; 3]; 6]> {
    curves
        .iter()
        .map(|c| c.map(|p| p.map(f64::to_bits)))
        .collect()
}

enum Attempt {
    Accepted(Candidate),
    Rejected(&'static str),
    Budget,
}

fn attempt(
    curves: &[Control],
    tables: &[Option<Vec<f64>>],
    angle: bool,
    n: u32,
    input: &EdgeInput,
    budget: &mut Budget,
) -> Attempt {
    if !budget.partition() {
        return Attempt::Budget;
    }
    let mut sink = EmissionSink::new(input.points[0]);
    let mut parameter_bits = Vec::new();
    for (c, table) in curves.iter().zip(tables) {
        let params = if angle {
            let Some(table) = table else {
                return Attempt::Rejected("angle_partition_unavailable");
            };
            split_params(table, n)
        } else {
            (0..=n).map(|i| f64::from(i) / f64::from(n)).collect()
        };
        let cubic = [c[0], c[1], c[2], c[3]];
        let eval = |t| {
            if t == 0.0 {
                c[0]
            } else if t == 1.0 {
                c[3]
            } else {
                cubic_at(c[0], c[1], c[2], c[3], t)
            }
        };
        for w in params.windows(2) {
            // 两个端点使用、四个控制差评估及一次 sink 点使用均计入。
            if !budget.sample(7) {
                return Attempt::Budget;
            }
            let from = quantize_vec3(eval(w[0])).map(f64::from);
            let end = eval(w[1]);
            let to = quantize_vec3(end).map(f64::from);
            if quality::chord_error(restricted(cubic, w[0], w[1]), from, to) > 0.01 {
                return Attempt::Rejected("sampling_deviation");
            }
            if sink.push(end).is_err() {
                return Attempt::Rejected("sink");
            }
        }
        parameter_bits.push(params.iter().map(|t| t.to_bits()).collect::<Vec<_>>());
    }
    let arc = sink.total_arc_meters;
    let program =
        re::RoadEditingCurveProgram::try_new(point3(input.points[0]).unwrap(), sink.segments)
            .unwrap();
    let points = program_points(&program);
    let dirs = [
        unit(sub(points[1], points[0])).unwrap(),
        unit(sub(points[points.len() - 1], points[points.len() - 2])).unwrap(),
    ];
    if (0..2)
        .any(|i| input.clamps[i].is_some_and(|d| angle_rad(dirs[i], d).to_degrees() > END_DEGREES))
    {
        return Attempt::Rejected("complete_end_direction");
    }
    Attempt::Accepted(Candidate {
        repaired: RepairedCurve {
            program,
            quantized_arc_meters: arc,
        },
        spans: vec![
            json!({"control_f64_bits":bits(curves),"parameter_f64_bits":parameter_bits,
            "partition":if angle {"tangent_angle"} else {"uniform"},"pieces_per_cubic":n}),
        ],
    })
}

struct Ranked {
    score: [f64; 3],
    order: [usize; 3],
    candidate: Candidate,
    curves: Vec<Control>,
}

fn rank_compare(a: &Ranked, b: &Ranked) -> std::cmp::Ordering {
    for (a, b) in a.score.iter().zip(b.score) {
        let order = a.total_cmp(&b);
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    a.order.cmp(&b.order)
}

pub(crate) fn policy() -> Value {
    json!({"geometry_attempts_per_edge":148,"handle_factors":FACTORS,
        "seams":["keep","pair","whole"],"pieces_per_cubic":[2,MAX_PIECES],
        "partitions_per_edge":PARTITION_LIMIT,"sampling_work":WORK_LIMIT,"source_fit_distance_work":WORK_LIMIT,
        "source_fit_steps":64,"tangent_scan_steps":SCAN_STEPS,"reference_cubics_max":MAX_CURVES,
        "program_pool":PROGRAM_LIMIT,"dedup_point_limit":FIT_KEY_POINT_LIMIT,
        "sampling_deviation_m":0.01,"end_degrees":END_DEGREES,
        "source_fit_screen":"both directional upper estimates <= original reference under the same algorithm",
        "sampling_work_unit":"2 point uses + 4 subcurve control differences + 1 sink point use per attempted interval; scan tangent evaluations",
        "partition_stop":"first accepted partition per geometry and family",
        "scope":"test research; ordinary f64 estimates are not continuous certificates or production acceptance"})
}

pub(crate) fn search(input: &EdgeInput) -> EdgeSearch {
    assert!(input.points.len() >= 2);
    assert!(
        input
            .points
            .iter()
            .flatten()
            .all(|x| x.is_finite() && x.abs() <= 32_000.0)
    );
    assert!(
        input
            .clamps
            .iter()
            .all(|c| c.is_some_and(|d| (norm(d) - 1.0).abs() < 1e-12)),
        "unit end clamps"
    );
    let original = reference(input);
    let mut budget = Budget::default();
    let reference_fit =
        quality::fit(&original, &input.points, &mut budget).expect("bounded reference fit");
    let reference_length = quality::lengths(&original);
    let reference_arc = reference_length["chord_estimate_m"].as_f64().unwrap();
    let mut rows = Vec::new();
    let mut pool = Vec::<Ranked>::new();
    let mut seen = ExactSeen::default();
    for (index, geometry) in geometries(&original).into_iter().enumerate() {
        let curves = geometry.curves;
        let mut row = json!({"geometry":index,"description":geometry.description,"controls_f64_bits":bits(&curves),
            "status":"pending","partition_rejections":{},"successful_partitions":[]});
        if !monotone(&curves) {
            row["status"] = json!("backtracking_control_rejected");
            rows.push(row);
            continue;
        }
        let Some(fit) = quality::fit(&curves, &input.points, &mut budget) else {
            row["status"] = json!("budget_stopped");
            rows.push(row);
            break;
        };
        row["source_fit"] = json!(fit);
        if !fit.within(reference_fit) {
            row["status"] = json!("source_fit_rejected");
            rows.push(row);
            continue;
        }
        let mut tables = Vec::new();
        for c in &curves {
            if !budget.sample((SCAN_STEPS + 1) as u64) {
                break;
            }
            tables.push(
                scan(&BezierSpan {
                    a: c[0],
                    c1: c[1],
                    c2: c[2],
                    b: c[3],
                    dir_a: c[4],
                    dir_b: c[5],
                })
                .0,
            );
        }
        let mut rejections = BTreeMap::<&str, u64>::new();
        let mut successes = Vec::new();
        for (family, angle) in [(0, false), (1, true)] {
            if budget.stop.is_some() {
                break;
            }
            for n in 2..=MAX_PIECES {
                match attempt(&curves, &tables, angle, n, input, &mut budget) {
                    Attempt::Budget => break,
                    Attempt::Rejected(reason) => {
                        *rejections.entry(reason).or_default() += 1;
                    }
                    Attempt::Accepted(mut candidate) => {
                        candidate.spans[0]["geometry"] = json!(index);
                        candidate.spans[0]["source_fit"] = json!(fit);
                        successes.push(json!({"family":family,"n":n}));
                        let key =
                            canonical_key(program_points(&candidate.repaired.program).into_iter());
                        match seen.insert(key, FIT_KEY_POINT_LIMIT) {
                            Retain::Duplicate => {}
                            Retain::Budget => {
                                budget.stop = Some("dedup_points");
                            }
                            Retain::New => {
                                let score = fit.score();
                                pool.push(Ranked {
                                    score: [
                                        score[0],
                                        score[1],
                                        (candidate.repaired.quantized_arc_meters - reference_arc)
                                            .abs(),
                                    ],
                                    order: [index, family, n as usize],
                                    candidate,
                                    curves: curves.clone(),
                                });
                                pool.sort_by(rank_compare);
                                pool.truncate(PROGRAM_LIMIT);
                            }
                        }
                        budget.retained_key_points = seen.points;
                        break;
                    }
                }
            }
        }
        row["partition_rejections"] = json!(rejections);
        row["successful_partitions"] = json!(successes);
        row["status"] = json!(if budget.stop.is_some() {
            "budget_stopped"
        } else if successes.is_empty() {
            "no_partition_in_family"
        } else {
            "candidate_found"
        });
        rows.push(row);
        if budget.stop.is_some() {
            break;
        }
    }
    let candidates = pool
        .into_iter()
        .map(|mut p| {
            p.candidate.spans[0]["reference_length_estimates"] = quality::lengths(&p.curves);
            let canonical_fit = quality::polyline_fit(
                &program_points(&p.candidate.repaired.program),
                &input.points,
                &mut budget,
            );
            p.candidate.spans[0]["canonical_source_fit"] = json!(canonical_fit);
            p.candidate
        })
        .collect::<Vec<_>>();
    let report = json!({"policy":policy(),"reference_controls_f64_bits":bits(&original),
        "reference_fit":reference_fit,"reference_length_estimates":reference_length,
        "source_shape_polyline_length_m":input.points.windows(2).map(|w|norm(sub(w[1],w[0]))).sum::<f64>(),
        "counts":budget,"geometries":rows,"geometry_family_visited":rows.len()==148 && budget.stop.is_none(),
        "budget_truncated":budget.stop.is_some(),"stop_reason":budget.stop.unwrap_or("completed_finite_geometry_family"),
        "candidate_programs":candidates.len(),"production_g1_accepted":false});
    EdgeSearch { candidates, report }
}
