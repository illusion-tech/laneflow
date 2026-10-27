use super::*;

#[derive(Clone, Copy)]
struct Chord {
    end: usize,
    dir: Vec3,
}

struct Node {
    from: usize,
    to: usize,
    parent: Option<usize>,
    dir: Vec3,
    depth: usize,
}

fn path(nodes: &[Node], mut at: usize) -> Vec<usize> {
    let mut result = vec![nodes[at].to];
    loop {
        result.push(nodes[at].from);
        let Some(parent) = nodes[at].parent else {
            break;
        };
        at = parent;
    }
    result.reverse();
    result
}

pub(super) fn deviation(c: [Vec3; 4], ta: f64, tb: f64, a: Vec3, b: Vec3) -> f64 {
    let curve = restricted(c, ta, tb);
    let line = [
        a,
        add(a, scale(sub(b, a), 1.0 / 3.0)),
        add(a, scale(sub(b, a), 2.0 / 3.0)),
        b,
    ];
    (0..4)
        .map(|k| norm(sub(curve[k], line[k])))
        .fold(0.0, f64::max)
}

pub(super) fn paths(
    state: &mut GridSearch,
    control: [Vec3; 6],
    incoming: Option<Vec3>,
) -> Result<Vec<Vec<usize>>> {
    let params = (0..=STEPS)
        .map(|i| i as f64 / STEPS as f64)
        .collect::<Vec<_>>();
    paths_on(state, control, incoming, &params)
}

pub(super) fn paths_on(
    state: &mut GridSearch,
    control: [Vec3; 6],
    incoming: Option<Vec3>,
    params: &[f64],
) -> Result<Vec<Vec<usize>>> {
    assert_eq!(params.len(), STEPS + 1);
    assert_eq!(params[0], 0.0);
    assert_eq!(params[STEPS], 1.0);
    assert!(params.windows(2).all(|p| p[0] < p[1]));
    state.counts.graph_searches += 1;
    state.reserve_work((STEPS + 1) as u64)?;
    let c = [control[0], control[1], control[2], control[3]];
    let points = (0..=STEPS)
        .map(|i| {
            quantize_vec3(if i == 0 {
                c[0]
            } else if i == STEPS {
                c[3]
            } else {
                cubic_at(c[0], c[1], c[2], c[3], params[i])
            })
        })
        .collect::<Vec<_>>();
    state.counts.node_evaluations += (STEPS + 1) as u64;
    let mut outgoing = vec![Vec::new(); STEPS + 1];
    let mut chords = 0;
    for i in 0..STEPS {
        for j in (i + 1..=STEPS).rev() {
            state.reserve_work(1)?;
            state.counts.chord_checks += 1;
            if quantized_chord_meters(points[i], points[j]) <= QUANTIZED_MIN_SEGMENT_METERS {
                continue;
            }
            let a = points[i].map(f64::from);
            let b = points[j].map(f64::from);
            if deviation(c, params[i], params[j], a, b) > DEVIATION_METERS {
                continue;
            }
            outgoing[i].push(Chord {
                end: j,
                dir: unit(sub(b, a)).unwrap(),
            });
            chords += 1;
            state.counts.chords_peak = state.counts.chords_peak.max(chords);
        }
    }
    state.counts.chords_peak = state.counts.chords_peak.max(chords);
    let mut nodes = Vec::<Node>::new();
    let outdegrees = outgoing.iter().map(Vec::len).collect::<Vec<_>>();
    for chord in &outgoing[0] {
        state.reserve_work(1)?;
        state.counts.transition_checks += 1;
        if incoming.is_none()
            && state
                .start_clamp
                .is_some_and(|d| angle_rad(d, chord.dir).to_degrees() > END_DEGREES)
        {
            state.counts.start_clamp_rejected += 1;
            continue;
        }
        if incoming.is_some_and(|d| angle_rad(d, chord.dir) > WELD_MAX_TURN_RAD) {
            continue;
        }
        nodes.push(Node {
            from: 0,
            to: chord.end,
            parent: None,
            dir: chord.dir,
            depth: 1,
        });
        state.counts.states_peak = state.counts.states_peak.max(nodes.len());
    }
    let seed_count = nodes.len();
    let mut min_rejected_turn: Option<f64> = None;
    let mut min_terminal_turn: Option<f64> = None;
    let mut cursor = 0;
    let mut limited = false;
    while cursor < nodes.len() && !limited {
        let node = &nodes[cursor];
        let from = node.to;
        let dir = node.dir;
        let depth = node.depth + 1;
        state.counts.previously_reached_chords_skipped +=
            (outdegrees[from] - outgoing[from].len()) as u64;
        let mut error = None;
        // 相同有向弦的后续可行性只取决于其末点和方向；首个 BFS 前缀保留后即可移除。
        outgoing[from].retain(|chord| {
            if limited || error.is_some() {
                return true;
            }
            if let Err(e) = state.reserve_work(1) {
                error = Some(e);
                return true;
            }
            state.counts.transition_checks += 1;
            let angle = angle_rad(dir, chord.dir);
            if chord.end == STEPS {
                min_terminal_turn = Some(min_terminal_turn.map_or(angle, |a| a.min(angle)));
            }
            if angle > WELD_MAX_TURN_RAD {
                min_rejected_turn = Some(min_rejected_turn.map_or(angle, |a| a.min(angle)));
                return true;
            }
            if nodes.len() == STATE_LIMIT {
                limited = true;
                return true;
            }
            nodes.push(Node {
                from,
                to: chord.end,
                parent: Some(cursor),
                dir: chord.dir,
                depth,
            });
            state.counts.states_peak = state.counts.states_peak.max(nodes.len());
            false
        });
        if let Some(error) = error {
            return Err(error);
        }
        cursor += 1;
    }
    state.counts.state_limited_searches += u64::from(limited);
    state.counts.states_peak = state.counts.states_peak.max(nodes.len());
    let mut terminal = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.to == STEPS)
        .map(|(i, n)| (angle_rad(n.dir, control[5]), i))
        .collect::<Vec<_>>();
    if let Some(audit) = &mut state.audit {
        audit.graph(
            json!({"control_f64_bits":control.map(|p|p.map(f64::to_bits)),
            "incoming_dir_f64_bits":incoming.map(|d|d.map(f64::to_bits)),
            "seed_states":seed_count,"states":nodes.len(),"terminals":terminal.len(),
            "state_limit":limited,"maximum_reached_index":nodes.iter().map(|n|n.to).max(),
            "minimum_terminal_turn_degrees":min_terminal_turn.map(f64::to_degrees),
            "minimum_rejected_turn_degrees":min_rejected_turn.map(f64::to_degrees)}),
        );
    }
    terminal.sort_by(|(aa, a), (bb, b)| {
        aa.total_cmp(bb)
            .then(nodes[*a].depth.cmp(&nodes[*b].depth))
            .then_with(|| path(&nodes, *a).cmp(&path(&nodes, *b)))
    });
    state.counts.representative_limited_searches +=
        u64::from(terminal.len() > REPRESENTATIVE_LIMIT);
    terminal.truncate(REPRESENTATIVE_LIMIT);
    state.reserve_points(terminal.iter().map(|(_, i)| nodes[*i].depth + 1).sum())?;
    Ok(terminal.into_iter().map(|(_, i)| path(&nodes, i)).collect())
}

#[test]
fn graph_paths_obey_real_sink_and_keep_exact_endpoints() {
    let c = [
        [0.0, 0.0, 0.0],
        [3.0, 0.3, 0.0],
        [7.0, 0.3, 0.0],
        [10.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let mut state = GridSearch::default();
    let found = paths(&mut state, c, None).unwrap();
    assert!(!found.is_empty());
    for p in found {
        assert_eq!(p.first(), Some(&0));
        assert_eq!(p.last(), Some(&STEPS));
        let mut sink = EmissionSink::new(c[0]);
        for k in p.into_iter().skip(1) {
            sink.push(if k == STEPS {
                c[3]
            } else {
                cubic_at(c[0], c[1], c[2], c[3], k as f64 / STEPS as f64)
            })
            .unwrap();
        }
        assert_eq!(sink.prev_point, quantize_vec3(c[3]));
    }
    assert!(state.work() <= EVALUATION_LIMIT);
}

#[test]
fn incoming_direction_and_degenerate_chords_are_not_relaxed() {
    let c = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    assert!(
        paths(&mut GridSearch::default(), c, Some([0.0, 1.0, 0.0]))
            .unwrap()
            .is_empty()
    );
    let short = c.map(|p| scale(p, 0.01));
    assert!(
        paths(&mut GridSearch::default(), short, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn first_chord_applies_original_end_clamp_before_state_merge() {
    let c = [
        [0.0, 0.0, 0.0],
        [3.0, 0.5, 0.0],
        [7.0, 0.5, 0.0],
        [10.0, 0.0, 0.0],
        unit([3.0, 0.5, 0.0]).unwrap(),
        unit([3.0, -0.5, 0.0]).unwrap(),
    ];
    let mut state = GridSearch {
        start_clamp: Some(c[4]),
        ..GridSearch::default()
    };
    let found = paths(&mut state, c, None).unwrap();
    assert!(!found.is_empty());
    for p in found {
        let first = quantize_vec3(cubic_at(c[0], c[1], c[2], c[3], p[1] as f64 / STEPS as f64))
            .map(f64::from);
        let dir = unit(sub(first, quantize_vec3(c[0]).map(f64::from))).unwrap();
        assert!(angle_rad(dir, c[4]).to_degrees() <= END_DEGREES);
    }
    assert!(state.counts.start_clamp_rejected > 0);
    let mut state = GridSearch {
        start_clamp: Some([0.0, 0.0, 1.0]),
        ..GridSearch::default()
    };
    assert!(paths(&mut state, c, None).unwrap().is_empty());
    assert!(paths(&mut state, c, Some(c[4])).is_ok());
}
