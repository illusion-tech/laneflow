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

fn deviation(c: [Vec3; 4], i: usize, j: usize, a: Vec3, b: Vec3) -> f64 {
    let curve = restricted(c, i as f64 / STEPS as f64, j as f64 / STEPS as f64);
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
                cubic_at(c[0], c[1], c[2], c[3], i as f64 / STEPS as f64)
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
            if deviation(c, i, j, a, b) > DEVIATION_METERS {
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
    let mut seen = BTreeSet::new();
    for chord in &outgoing[0] {
        state.reserve_work(1)?;
        state.counts.transition_checks += 1;
        if incoming.is_some_and(|d| angle_rad(d, chord.dir) > WELD_MAX_TURN_RAD) {
            continue;
        }
        seen.insert((0, chord.end));
        nodes.push(Node {
            from: 0,
            to: chord.end,
            parent: None,
            dir: chord.dir,
            depth: 1,
        });
        state.counts.states_peak = state.counts.states_peak.max(nodes.len());
    }
    let mut cursor = 0;
    let mut limited = false;
    while cursor < nodes.len() && !limited {
        let node = &nodes[cursor];
        let from = node.to;
        let dir = node.dir;
        let depth = node.depth + 1;
        for chord in &outgoing[from] {
            state.reserve_work(1)?;
            state.counts.transition_checks += 1;
            if angle_rad(dir, chord.dir) > WELD_MAX_TURN_RAD {
                continue;
            }
            if seen.contains(&(from, chord.end)) {
                state.counts.merged_states += 1;
                continue;
            }
            if nodes.len() == STATE_LIMIT {
                limited = true;
                break;
            }
            seen.insert((from, chord.end));
            nodes.push(Node {
                from,
                to: chord.end,
                parent: Some(cursor),
                dir: chord.dir,
                depth,
            });
            state.counts.states_peak = state.counts.states_peak.max(nodes.len());
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
