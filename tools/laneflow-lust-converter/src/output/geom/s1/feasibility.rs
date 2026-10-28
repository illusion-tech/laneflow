//! #1113：首弦/末两弦的保守必要条件排除与真实局部见证，不修改生产搜索。

use super::*;
use interval::{Interval, angle_excluded, curve, difference, point, short};

mod interval;
mod seam;

pub(crate) use seam::diagnose as diagnose_seam;

const BOX_LIMIT: u64 = 32_000_000;
const DEPTH_LIMIT: u32 = 48;

fn constant_point(b: interval::Box2) -> Option<Vec3> {
    (b.iter().all(|x| x.lo == x.hi)).then_some([b[0].lo, 0.0, b[1].lo])
}

fn constant_rejection(
    a: interval::Box2,
    b: interval::Box2,
    end: interval::Box2,
    clamp: Option<Vec3>,
) -> bool {
    let (Some(a), Some(b), Some(end)) = (constant_point(a), constant_point(b), constant_point(end))
    else {
        return false;
    };
    if quantized_chord_meters(quantize_vec3(a), quantize_vec3(b)) <= QUANTIZED_MIN_SEGMENT_METERS {
        return true;
    }
    let dir = unit(sub(b, a)).unwrap();
    if let Some(clamp) = clamp {
        return angle_rad(clamp, dir).to_degrees() > END_DEGREES;
    }
    if quantized_chord_meters(quantize_vec3(b), quantize_vec3(end)) <= QUANTIZED_MIN_SEGMENT_METERS
    {
        return true;
    }
    angle_rad(dir, unit(sub(end, b)).unwrap()) > WELD_MAX_TURN_RAD
}

fn at(c: [Vec3; 4], t: f64) -> Vec3 {
    if t == 0.0 {
        c[0]
    } else if t == 1.0 {
        c[3]
    } else {
        cubic_at(c[0], c[1], c[2], c[3], t)
    }
}

fn witness(c: [Vec3; 4], params: &[f64], clamp: Option<Vec3>) -> Option<Value> {
    if !params.windows(2).all(|p| p[0] < p[1]) {
        return None;
    }
    let points = params.iter().map(|&t| at(c, t)).collect::<Vec<_>>();
    let q = points
        .iter()
        .copied()
        .map(quantize_vec3)
        .collect::<Vec<_>>();
    let mut sink = EmissionSink::new(points[0]);
    for (i, &p) in points.iter().enumerate().skip(1) {
        if grid::deviation_for_probe(
            c,
            params[i - 1],
            params[i],
            q[i - 1].map(f64::from),
            q[i].map(f64::from),
        ) > 0.01
        {
            return None;
        }
        sink.push(p).ok()?;
    }
    if clamp.is_some_and(|d| {
        angle_rad(
            d,
            unit(sub(q[1].map(f64::from), q[0].map(f64::from))).unwrap(),
        )
        .to_degrees()
            > END_DEGREES
    }) {
        return None;
    }
    Some(
        json!({"parameters_f64_bits":params.iter().map(|t|t.to_bits()).collect::<Vec<_>>(),
        "points_f32_bits":q.iter().map(|p|p.map(f32::to_bits)).collect::<Vec<_>>(),
        "chord_meters":q.windows(2).map(|p|quantized_chord_meters(p[0],p[1])).collect::<Vec<_>>(),
        "scope":"local necessary-condition witness only"}),
    )
}

pub(crate) fn diagnose(control: [Vec3; 6], clamp: Option<Vec3>) -> Value {
    let c = [control[0], control[1], control[2], control[3]];
    assert!(
        c.iter()
            .all(|p| p[1] == 0.0 && p.iter().all(|x| x.is_finite() && x.abs() <= 32_000.0))
    );
    if let Some(d) = clamp {
        assert_eq!(d[1], 0.0);
        assert!((norm(d) - 1.0).abs() <= 1e-12, "unit clamp required");
    }
    let end = point(quantize_vec3(c[3]));
    let start = point(quantize_vec3(c[0]));
    let domain = Interval { lo: 0.0, hi: 1.0 };
    let mut stack = vec![(domain, domain, 0_u32)];
    let mut counts = BTreeMap::<&str, u64>::new();
    let mut visited = 0;
    let mut deepest = 0;
    let mut found = None;
    let mut unresolved = Vec::new();
    while let Some((u, v, depth)) = stack.pop() {
        if visited == BOX_LIMIT {
            stack.push((u, v, depth));
            break;
        }
        visited += 1;
        deepest = deepest.max(depth);
        let a = if clamp.is_some() { start } else { curve(c, u) };
        let b = curve(c, v);
        let first = difference(b, a);
        let second = difference(end, b);
        let reason = if clamp.is_none() && u.lo >= v.hi {
            Some("parameter_order")
        } else if constant_rejection(a, b, end, clamp) {
            Some("constant_quantized_rejection")
        } else if short(first) || (clamp.is_none() && short(second)) {
            Some("minimum_chord")
        } else if let Some(d) = clamp {
            let direction = [Interval::point(d[0]), Interval::point(d[2])];
            angle_excluded(direction, first, 0.0166).then_some("start_direction")
        } else {
            angle_excluded(first, second, 0.0341).then_some("terminal_turn")
        };
        if let Some(reason) = reason {
            *counts.entry(reason).or_default() += 1;
            continue;
        }
        let um = (u.lo + u.hi) * 0.5;
        let vm = (v.lo + v.hi) * 0.5;
        let params = if clamp.is_some() {
            vec![0.0, vm]
        } else {
            vec![um, vm, 1.0]
        };
        if let Some(candidate) = witness(c, &params, clamp) {
            found = Some(candidate);
            break;
        }
        if depth == DEPTH_LIMIT {
            *counts.entry("unresolved_depth").or_default() += 1;
            if unresolved.len() < 16 {
                unresolved.push(json!({"u":[u.lo,u.hi],"v":[v.lo,v.hi]}));
            }
            continue;
        }
        if clamp.is_none() && u.hi - u.lo > v.hi - v.lo {
            stack.push((Interval { lo: um, hi: u.hi }, v, depth + 1));
            stack.push((Interval { lo: u.lo, hi: um }, v, depth + 1));
        } else {
            stack.push((u, Interval { lo: vm, hi: v.hi }, depth + 1));
            stack.push((u, Interval { lo: v.lo, hi: vm }, depth + 1));
        }
    }
    let excluded = found.is_none() && stack.is_empty() && unresolved.is_empty();
    json!({"control_f64_bits":control.map(|p|p.map(f64::to_bits)),"clamp_f64_bits":clamp.map(|d|d.map(f64::to_bits)),
        "kind":if clamp.is_some(){"first_chord"}else{"last_two_chords"},
        "status":if found.is_some(){"local_witness"}else if excluded{"necessary_condition_excluded"}else{"unresolved"},
        "witness":found,"visited_boxes":visited,"deepest":deepest,"excluded_boxes":counts,"pending_boxes":stack.len(),
        "first_unresolved_boxes":unresolved,"single_chord_witness":witness(c,&[0.0,1.0],clamp),
        "single_chord_deviation_m":grid::deviation_for_probe(c,0.0,1.0,quantize_vec3(c[0]).map(f64::from),quantize_vec3(c[3]).map(f64::from)),
        "policy":{"box_limit":BOX_LIMIT,"depth_limit":DEPTH_LIMIT,"length_exclusion_m":0.099_999,"start_tangent_upper":0.0166,"turn_tangent_upper":0.0341,
        "roundoff_epsilon_factor":128,"direction_and_length_exclusion_are_relaxed":true,"deviation_omitted_from_exclusion":true},
        "scope":"fixed planar cubic endpoint necessary conditions; not a continuous approximation-error certificate or complete edge acceptance"})
}

#[test]
fn straight_curve_is_not_falsely_excluded() {
    let c = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    assert_eq!(diagnose(c, Some(c[4]))["status"], "local_witness");
    assert_eq!(diagnose(c, None)["status"], "local_witness");
    assert_eq!(
        diagnose(c, Some([0.0, 0.0, 1.0]))["status"],
        "necessary_condition_excluded"
    );
}
