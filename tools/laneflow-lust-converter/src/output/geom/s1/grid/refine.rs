//! #1111：在固定近失配路径各内部参数的邻域中搜索；端点和 cubic 不变。

use super::*;

const MAX_KNOTS: usize = 32;
const HALF_WINDOW: i32 = 32;
const DENOMINATOR: f64 = 8_192.0;

struct Node {
    point: usize,
    layer: usize,
    dir: Vec3,
    parent: Option<usize>,
}

fn parameters(layers: &[Vec<f64>], nodes: &[Node], mut at: usize) -> Vec<f64> {
    let mut p = Vec::new();
    loop {
        let node = &nodes[at];
        p.push(layers[node.layer][node.point]);
        let Some(parent) = node.parent else {
            break;
        };
        at = parent;
    }
    p.push(0.0);
    p.reverse();
    p
}

pub(super) fn search(
    state: &mut GridSearch,
    control: [Vec3; 6],
    incoming: Option<Vec3>,
    skeleton: &[f64],
) -> Result<Vec<Vec<f64>>> {
    state.counts.refinement_calls += 1;
    if skeleton.len() > MAX_KNOTS {
        state.counts.refinement_skeleton_limited += 1;
        return Ok(Vec::new());
    }
    assert!(skeleton.len() >= 2);
    assert_eq!(skeleton[0], 0.0);
    assert_eq!(*skeleton.last().unwrap(), 1.0);
    let c = [control[0], control[1], control[2], control[3]];
    let layers = skeleton
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            if i == 0 || i + 1 == skeleton.len() {
                return vec![t];
            }
            (-HALF_WINDOW..=HALF_WINDOW)
                .map(|k| t + f64::from(k) / DENOMINATOR)
                .filter(|&t| t > 0.0 && t < 1.0)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut points = Vec::new();
    for layer in &layers {
        let mut values = Vec::new();
        for &t in layer {
            state.reserve_work(1)?;
            state.counts.node_evaluations += 1;
            values.push(quantize_vec3(if t == 0.0 {
                c[0]
            } else if t == 1.0 {
                c[3]
            } else {
                cubic_at(c[0], c[1], c[2], c[3], t)
            }));
        }
        points.push(values);
    }
    let mut chords = Vec::new();
    let mut chord_count = 0;
    for k in 0..layers.len() - 1 {
        let mut matrix = Vec::new();
        for (i, &ta) in layers[k].iter().enumerate() {
            for (j, &tb) in layers[k + 1].iter().enumerate() {
                state.reserve_work(1)?;
                state.counts.chord_checks += 1;
                let a = points[k][i];
                let b = points[k + 1][j];
                let valid = ta < tb
                    && quantized_chord_meters(a, b) > QUANTIZED_MIN_SEGMENT_METERS
                    && graph::deviation(c, ta, tb, a.map(f64::from), b.map(f64::from))
                        <= DEVIATION_METERS;
                let dir = valid.then(|| unit(sub(b.map(f64::from), a.map(f64::from))).unwrap());
                chord_count += usize::from(valid);
                matrix.push(dir);
            }
        }
        chords.push(matrix);
    }
    state.counts.refinement_chords_peak = state.counts.refinement_chords_peak.max(chord_count);
    let mut nodes = Vec::new();
    for (point, &dir) in chords[0].iter().enumerate() {
        let Some(dir) = dir else {
            continue;
        };
        state.reserve_work(1)?;
        state.counts.transition_checks += 1;
        if incoming.is_some_and(|d| angle_rad(d, dir) > WELD_MAX_TURN_RAD)
            || (incoming.is_none()
                && state
                    .start_clamp
                    .is_some_and(|d| angle_rad(d, dir).to_degrees() > END_DEGREES))
        {
            continue;
        }
        nodes.push(Node {
            point,
            layer: 1,
            dir,
            parent: None,
        });
    }
    let mut frontier = (0..nodes.len()).collect::<Vec<_>>();
    state.counts.refinement_states_peak = state.counts.refinement_states_peak.max(nodes.len());
    for layer in 1..layers.len() - 1 {
        let mut reached = vec![false; chords[layer].len()];
        let mut next = Vec::new();
        for at in frontier {
            let current = nodes[at].point;
            let dir = nodes[at].dir;
            for point in 0..layers[layer + 1].len() {
                let slot = current * layers[layer + 1].len() + point;
                if reached[slot] {
                    state.counts.previously_reached_chords_skipped += 1;
                    continue;
                }
                let Some(target) = chords[layer][slot] else {
                    continue;
                };
                state.reserve_work(1)?;
                state.counts.transition_checks += 1;
                if angle_rad(dir, target) > WELD_MAX_TURN_RAD {
                    continue;
                }
                if nodes.len() == STATE_LIMIT {
                    state.counts.state_limited_searches += 1;
                    return Ok(Vec::new());
                }
                reached[slot] = true;
                next.push(nodes.len());
                nodes.push(Node {
                    point,
                    layer: layer + 1,
                    dir: target,
                    parent: Some(at),
                });
            }
        }
        frontier = next;
        state.counts.refinement_states_peak = state.counts.refinement_states_peak.max(nodes.len());
        if frontier.is_empty() {
            break;
        }
    }
    frontier.sort_by(|&a, &b| {
        angle_rad(nodes[a].dir, control[5])
            .total_cmp(&angle_rad(nodes[b].dir, control[5]))
            .then_with(|| {
                parameters(&layers, &nodes, a)
                    .iter()
                    .map(|t| t.to_bits())
                    .cmp(parameters(&layers, &nodes, b).iter().map(|t| t.to_bits()))
            })
    });
    state.counts.representative_limited_searches +=
        u64::from(frontier.len() > REPRESENTATIVE_LIMIT);
    frontier.truncate(REPRESENTATIVE_LIMIT);
    Ok(frontier
        .into_iter()
        .map(|i| parameters(&layers, &nodes, i))
        .collect())
}
