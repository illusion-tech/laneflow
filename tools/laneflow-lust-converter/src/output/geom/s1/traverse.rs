use super::*;

pub(super) fn sample(
    state: &mut Search,
    span_index: usize,
    control: [Vec3; 6],
    sink: &mut EmissionSink,
) -> Result<()> {
    let index = state.encountered;
    state.encountered += 1;
    let [a, c1, c2, b, dir_a, dir_b] = control;
    let span = BezierSpan {
        a,
        c1,
        c2,
        b,
        dir_a,
        dir_b,
    };
    let checkpoint = sink.checkpoint();
    assert_eq!(checkpoint.prev_point, quantize_vec3(a), "fixed span seam");
    if index == state.frames.len() {
        if state.frames.len() == CACHED_CUBIC_LIMIT {
            return Err(state.truncate("frames"));
        }
        state.frames.push(Frame {
            control: control.map(|p| p.map(f64::to_bits)),
            incoming_point: checkpoint.prev_point.map(f32::to_bits),
            incoming_dir: checkpoint.prev_dir.map(|p| p.map(f64::to_bits)),
            next_partition: 0,
            rank: 0,
            seen: ExactSeen::default(),
            selected: None,
        });
        state.counts.frames_peak = state.counts.frames_peak.max(state.frames.len());
    }
    let frame = &state.frames[index];
    assert_eq!(
        frame.control,
        control.map(|p| p.map(f64::to_bits)),
        "fixed frame geometry"
    );
    assert_eq!(
        frame.incoming_point,
        checkpoint.prev_point.map(f32::to_bits),
        "fixed frame point"
    );
    assert_eq!(
        frame.incoming_dir,
        checkpoint.prev_dir.map(|p| p.map(f64::to_bits)),
        "fixed frame direction"
    );
    if let Some(saved) = &frame.selected {
        if state.work() + saved.points.len() as u64 > EVALUATION_LIMIT {
            return Err(state.truncate("evaluations"));
        }
        state.counts.prefix_reuses += 1;
        for &point in &saved.points {
            state.counts.prefix_point_uses += 1;
            state.counts.cubic_sink_pushes += 1;
            // 仅复用坐标和诊断，仍由原 sink 对实际前缀逐点检查。
            if let Err(error) = sink.push(point) {
                sink.restore(&checkpoint);
                return Err(error);
            }
        }
        state.spans.push(saved.report.clone());
        return Ok(());
    }
    while state.frames[index].next_partition < (SAMPLE_MAX_PIECES - 1) * 2 {
        let ordinal = state.frames[index].next_partition;
        let n = ordinal / 2 + 2;
        let angle = ordinal % 2 == 1;
        let new_scan = angle && !state.scans.contains_key(&index);
        if state.counts.partitions == PARTITION_LIMIT {
            return Err(state.truncate("partitions"));
        }
        if state.work() + u64::from(n) + if new_scan { 513 } else { 0 } > EVALUATION_LIMIT {
            return Err(state.truncate("evaluations"));
        }
        state.frames[index].next_partition += 1;
        state.counts.partitions += 1;
        let params = if angle {
            if new_scan {
                let (cumulative, evaluations) = scan(&span);
                state.counts.tangent_evaluations += evaluations;
                state.counts.scan_builds += 1;
                state.scans.insert(
                    index,
                    Scan {
                        control: control.map(|p| p.map(f64::to_bits)),
                        cumulative,
                    },
                );
                state.counts.cached_cubics_peak = state.scans.len();
                state.counts.scan_values_peak = state
                    .scans
                    .values()
                    .map(|s| s.cumulative.as_ref().map_or(0, Vec::len))
                    .sum();
            } else {
                state.counts.scan_reuses += 1;
            }
            let cached = &state.scans[&index];
            assert_eq!(
                cached.control,
                control.map(|p| p.map(f64::to_bits)),
                "fixed cubic cache input"
            );
            cached
                .cumulative
                .as_deref()
                .map(|values| split_params(values, n))
        } else {
            Some((0..=n).map(|k| f64::from(k) / f64::from(n)).collect())
        };
        let Some(params) = params.filter(|p| p.windows(2).all(|w| w[0] < w[1])) else {
            state.counts.unavailable_partition += 1;
            continue;
        };
        let mut passed = true;
        for (k, &t) in params.iter().enumerate().skip(1) {
            let end = if k == n as usize {
                b
            } else {
                cubic_at(a, c1, c2, b, t)
            };
            state.counts.point_evaluations += 1;
            state.counts.cubic_sink_pushes += 1;
            if sink.push(end).is_err() {
                passed = false;
                break;
            }
        }
        if !passed {
            state.counts.sink_rejected += 1;
            sink.restore(&checkpoint);
            continue;
        }
        let points = sink.segments[checkpoint.segments_len..]
            .iter()
            .map(|s| {
                let re::RoadEditingCurveSegmentGeometry::Line { end } = s.geometry() else {
                    panic!("line program");
                };
                [end.x(), end.y(), end.z()]
            })
            .collect::<Vec<_>>();
        let key = canonical_key(std::iter::once(a).chain(points.iter().copied()));
        let other_keys = state.key_points() - state.frames[index].seen.points;
        match state.frames[index]
            .seen
            .insert(key, RETAINED_KEY_POINT_LIMIT - other_keys)
        {
            Retain::Duplicate => {
                state.counts.duplicate_partitions += 1;
                sink.restore(&checkpoint);
                continue;
            }
            Retain::Budget => {
                sink.restore(&checkpoint);
                return Err(state.truncate("retained_keys"));
            }
            Retain::New => {}
        }
        state.counts.retained_key_points_peak = state
            .counts
            .retained_key_points_peak
            .max(state.key_points());
        if state.prefix_points() + points.len() > RETAINED_PREFIX_POINT_LIMIT {
            sink.restore(&checkpoint);
            return Err(state.truncate("retained_prefix"));
        }
        let frame = &mut state.frames[index];
        let report = json!({"cubic_index":index, "span_index_zero_based":span_index, "pieces":n,
            "partition":if angle {"tangent_angle_512"} else {"uniform_t"},
            "candidate_rank":frame.rank, "control_f64_bits":([a,c1,c2,b].map(|p|p.map(f64::to_bits))),
            "metrics":metrics([a,c1,c2,b], &params)});
        frame.rank += 1;
        frame.selected = Some(SavedPrefix {
            points,
            report: report.clone(),
        });
        state.counts.retained_prefix_points_peak = state
            .counts
            .retained_prefix_points_peak
            .max(state.prefix_points());
        state.spans.push(report);
        return Ok(());
    }
    state.failed_at = Some(index);
    Err(Error::SumoModel(format!(
        "S1 有限划分族耗尽：cubic {index}"
    )))
}

fn gentle_control() -> [Vec3; 6] {
    [
        [0.0, 0.0, 0.0],
        [10.0, 0.1, 0.0],
        [20.0, 0.1, 0.0],
        [30.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ]
}

#[test]
fn retained_prefix_reuses_coordinates_but_rechecks_real_sink() {
    let mut state = Search::default();
    let control = gentle_control();
    let mut first = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut first).unwrap();
    let partitions = state.counts.partitions;
    let evaluations = state.counts.point_evaluations;
    state.encountered = 0;
    state.spans.clear();
    let mut again = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut again).unwrap();
    assert_eq!(state.counts.partitions, partitions);
    assert_eq!(state.counts.point_evaluations, evaluations);
    assert_eq!(state.counts.prefix_point_uses, first.segments.len() as u64);
    assert_eq!(
        again.total_arc_meters.to_bits(),
        first.total_arc_meters.to_bits()
    );
    assert_eq!(again.prev_dir, first.prev_dir);
    assert_eq!(again.segments.len(), first.segments.len());
    assert!(state.counts.cubic_sink_pushes > state.counts.point_evaluations);
}

#[test]
fn exhausted_suffix_can_advance_prefix_without_restarting_its_cursor() {
    let mut state = Search::default();
    let control = gentle_control();
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    let next = state.frames[0].next_partition;
    let mut impossible = control;
    impossible[0] = control[3];
    impossible[1] = impossible[0];
    impossible[2] = [30.05, 0.0, 0.0];
    impossible[3] = impossible[2];
    assert!(sample(&mut state, 1, impossible, &mut sink).is_err());
    assert_eq!(state.failed_at, Some(1));
    assert!(!state.truncated);
    state.frames.truncate(1);
    state.frames[0].selected = None;
    state.encountered = 0;
    let before = state.counts.partitions;
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    assert!(state.frames[0].next_partition > next);
    assert_eq!(state.counts.partitions - before, 2);
    assert_eq!(state.counts.duplicate_partitions, 1);
    assert_eq!(state.frames[0].rank, 2);
}

#[test]
#[should_panic(expected = "fixed frame direction")]
fn prefix_cache_cannot_cross_an_incoming_direction_change() {
    let mut state = Search::default();
    let control = gentle_control();
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    state.encountered = 0;
    let mut sink = EmissionSink::new(control[0]);
    sink.prev_dir = Some([0.0, 1.0, 0.0]);
    let _ = sample(&mut state, 0, control, &mut sink);
}

#[test]
fn prefix_replay_budget_stops_before_mutating_the_sink() {
    let mut state = Search::default();
    let control = gentle_control();
    let mut sink = EmissionSink::new(control[0]);
    sample(&mut state, 0, control, &mut sink).unwrap();
    state.encountered = 0;
    state.counts.point_evaluations = EVALUATION_LIMIT;
    let mut sink = EmissionSink::new(control[0]);
    assert!(sample(&mut state, 0, control, &mut sink).is_err());
    assert_eq!(state.truncation_reason, Some("evaluations"));
    assert!(sink.segments.is_empty());
    assert_eq!(sink.total_arc_meters, 0.0);
}
