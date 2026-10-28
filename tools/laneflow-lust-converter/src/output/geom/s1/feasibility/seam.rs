//! 固定相邻 cubic 接缝两侧的必要条件；不固定前一阶段选中的末弦。

use super::*;

fn witness(left: [Vec3; 4], right: [Vec3; 4], u: f64, v: f64) -> Option<Value> {
    if u >= 1.0 || v <= 0.0 {
        return None;
    }
    let points = [at(left, u), left[3], at(right, v)];
    let q = points.map(quantize_vec3);
    let deviations = [
        grid::deviation_for_probe(left, u, 1.0, q[0].map(f64::from), q[1].map(f64::from)),
        grid::deviation_for_probe(right, 0.0, v, q[1].map(f64::from), q[2].map(f64::from)),
    ];
    if deviations.into_iter().any(|d| d > 0.01) {
        return None;
    }
    let mut sink = EmissionSink::new(points[0]);
    sink.push(points[1]).ok()?;
    sink.push(points[2]).ok()?;
    Some(
        json!({"parameters_f64_bits":[u.to_bits(),v.to_bits()],"points_f32_bits":q.map(|p|p.map(f32::to_bits)),
        "chord_meters":[quantized_chord_meters(q[0],q[1]),quantized_chord_meters(q[1],q[2])],"deviation_estimates_m":deviations,
        "turn_degrees":angle_rad(unit(sub(q[1].map(f64::from),q[0].map(f64::from))).unwrap(),unit(sub(q[2].map(f64::from),q[1].map(f64::from))).unwrap()).to_degrees()}),
    )
}

pub(crate) fn diagnose(left: [Vec3; 4], right: [Vec3; 4]) -> Value {
    assert_eq!(left[3], right[0], "fixed seam");
    assert!(
        left.iter()
            .chain(&right)
            .all(|p| p[1] == 0.0 && p.iter().all(|x| x.is_finite() && x.abs() <= 32_000.0))
    );
    let joint = point(quantize_vec3(left[3]));
    let domain = Interval { lo: 0.0, hi: 1.0 };
    let mut stack = vec![(domain, domain, 0_u32)];
    let mut visited = 0;
    let mut deepest = 0;
    let mut counts = BTreeMap::<&str, u64>::new();
    let mut unresolved = Vec::new();
    let mut found = None;
    while let Some((u, v, depth)) = stack.pop() {
        if visited == BOX_LIMIT {
            stack.push((u, v, depth));
            break;
        }
        visited += 1;
        deepest = deepest.max(depth);
        let a = curve(left, u);
        let b = curve(right, v);
        let incoming = difference(joint, a);
        let outgoing = difference(b, joint);
        let reason = if constant_rejection(a, joint, b, None) {
            Some("constant_quantized_rejection")
        } else if short(incoming) || short(outgoing) {
            Some("minimum_chord")
        } else {
            angle_excluded(incoming, outgoing, 0.0341).then_some("seam_turn")
        };
        if let Some(reason) = reason {
            *counts.entry(reason).or_default() += 1;
            continue;
        }
        let um = (u.lo + u.hi) * 0.5;
        let vm = (v.lo + v.hi) * 0.5;
        if let Some(w) = witness(left, right, um, vm) {
            found = Some(w);
            break;
        }
        if depth == DEPTH_LIMIT {
            *counts.entry("unresolved_depth").or_default() += 1;
            if unresolved.len() < 16 {
                unresolved.push(json!({"u":[u.lo,u.hi],"v":[v.lo,v.hi]}));
            }
            continue;
        }
        if u.hi - u.lo > v.hi - v.lo {
            stack.push((Interval { lo: um, hi: u.hi }, v, depth + 1));
            stack.push((Interval { lo: u.lo, hi: um }, v, depth + 1));
        } else {
            stack.push((u, Interval { lo: vm, hi: v.hi }, depth + 1));
            stack.push((u, Interval { lo: v.lo, hi: vm }, depth + 1));
        }
    }
    let excluded = found.is_none() && stack.is_empty() && unresolved.is_empty();
    json!({"left_control_f64_bits":left.map(|p|p.map(f64::to_bits)),"right_control_f64_bits":right.map(|p|p.map(f64::to_bits)),
        "status":if found.is_some(){"local_witness"}else if excluded{"necessary_condition_excluded"}else{"unresolved"},
        "witness":found,"visited_boxes":visited,"deepest":deepest,"excluded_boxes":counts,"pending_boxes":stack.len(),"first_unresolved_boxes":unresolved,
        "policy":{"box_limit":BOX_LIMIT,"depth_limit":DEPTH_LIMIT,"turn_tangent_upper":0.0341,"length_exclusion_m":0.099_999,"roundoff_epsilon_factor":128,"deviation_omitted_from_exclusion":true},
        "scope":"all parameter pairs on two fixed adjacent planar cubics; mandatory fixed seam; excludes preceding/following chords and complete edge acceptance"})
}

#[test]
fn seam_search_retains_valid_join_and_excludes_right_angle() {
    let left = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
    ];
    let right = [
        [3.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [5.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
    ];
    assert_eq!(diagnose(left, right)["status"], "local_witness");
    let right = [
        [3.0, 0.0, 0.0],
        [3.0, 0.0, 1.0],
        [3.0, 0.0, 2.0],
        [3.0, 0.0, 3.0],
    ];
    assert_eq!(
        diagnose(left, right)["status"],
        "necessary_condition_excluded"
    );
}
