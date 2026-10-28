//! #1115：来源拟合与采样分别计量；普通 f64 上界估计不声明连续认证。

use super::*;

const STEPS: usize = 64;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct Fit {
    pub(super) curve_to_source_estimate_m: f64,
    pub(super) source_to_curve_estimate_m: f64,
}

impl Fit {
    pub(super) fn within(self, reference: Self) -> bool {
        self.curve_to_source_estimate_m <= reference.curve_to_source_estimate_m
            && self.source_to_curve_estimate_m <= reference.source_to_curve_estimate_m
    }

    pub(super) fn score(self) -> [f64; 2] {
        [
            self.curve_to_source_estimate_m
                .max(self.source_to_curve_estimate_m),
            self.curve_to_source_estimate_m + self.source_to_curve_estimate_m,
        ]
    }
}

fn distance(p: Vec3, a: Vec3, b: Vec3, budget: &mut Budget) -> Option<f64> {
    if !budget.quality(1) {
        return None;
    }
    Some(if a == b {
        norm(sub(p, a))
    } else {
        distance_to_chord(p, a, b)
    })
}

pub(super) fn fit(curves: &[Control], source: &[Vec3], budget: &mut Budget) -> Option<Fit> {
    assert!(source.windows(2).all(|w| norm(sub(w[1], w[0])) > 0.0));
    let mut forward = 0.0_f64;
    let mut reference_error = 0.0_f64;
    let mut chords = Vec::with_capacity(curves.len() * STEPS);
    for c in curves {
        let cubic = [c[0], c[1], c[2], c[3]];
        for k in 0..STEPS {
            let part = restricted(
                cubic,
                k as f64 / STEPS as f64,
                (k + 1) as f64 / STEPS as f64,
            );
            let mut best = f64::INFINITY;
            for w in source.windows(2) {
                let mut upper = 0.0_f64;
                for p in part {
                    upper = upper.max(distance(p, w[0], w[1], budget)?);
                }
                best = best.min(upper);
            }
            forward = forward.max(best);
            reference_error = reference_error.max(chord_error(part, part[0], part[3]));
            chords.push([part[0], part[3]]);
        }
    }
    let mut reverse = 0.0_f64;
    for w in source.windows(2) {
        for k in 0..STEPS {
            let a = add(w[0], scale(sub(w[1], w[0]), k as f64 / STEPS as f64));
            let b = add(w[0], scale(sub(w[1], w[0]), (k + 1) as f64 / STEPS as f64));
            let mut best = f64::INFINITY;
            for chord in &chords {
                let upper = distance(a, chord[0], chord[1], budget)?
                    .max(distance(b, chord[0], chord[1], budget)?);
                best = best.min(upper);
            }
            reverse = reverse.max(best);
        }
    }
    Some(Fit {
        curve_to_source_estimate_m: forward,
        source_to_curve_estimate_m: reverse + reference_error,
    })
}

pub(super) fn chord_error(part: [Vec3; 4], a: Vec3, b: Vec3) -> f64 {
    let line = [
        a,
        add(a, scale(sub(b, a), 1.0 / 3.0)),
        add(a, scale(sub(b, a), 2.0 / 3.0)),
        b,
    ];
    (0..4)
        .map(|i| norm(sub(part[i], line[i])))
        .fold(0.0, f64::max)
}

pub(super) fn polyline_fit(points: &[Vec3], source: &[Vec3], budget: &mut Budget) -> Option<Fit> {
    let directed = |a: &[Vec3], b: &[Vec3], steps: usize, budget: &mut Budget| -> Option<f64> {
        let mut worst = 0.0_f64;
        for w in a.windows(2) {
            for k in 0..steps {
                let left = add(w[0], scale(sub(w[1], w[0]), k as f64 / steps as f64));
                let right = add(w[0], scale(sub(w[1], w[0]), (k + 1) as f64 / steps as f64));
                let mut best = f64::INFINITY;
                for target in b.windows(2) {
                    best = best.min(
                        distance(left, target[0], target[1], budget)?
                            .max(distance(right, target[0], target[1], budget)?),
                    );
                }
                worst = worst.max(best);
            }
        }
        Some(worst)
    };
    Some(Fit {
        curve_to_source_estimate_m: directed(points, source, 1, budget)?,
        source_to_curve_estimate_m: directed(source, points, STEPS, budget)?,
    })
}

pub(super) fn lengths(curves: &[Control]) -> Value {
    let mut lower = 0.0;
    let mut upper = 0.0;
    for c in curves {
        for i in 0..4_096 {
            let part = restricted(
                [c[0], c[1], c[2], c[3]],
                f64::from(i) / 4_096.0,
                f64::from(i + 1) / 4_096.0,
            );
            lower += norm(sub(part[3], part[0]));
            upper += part.windows(2).map(|w| norm(sub(w[1], w[0]))).sum::<f64>();
        }
    }
    json!({"chord_estimate_m":lower,"control_polygon_estimate_m":upper,
        "intervals_per_cubic":4_096,"certified_continuous_bound":false})
}

#[test]
fn fit_measures_curve_interior_and_both_directions() {
    let c = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    let f = fit(&[c], &[c[0], c[3]], &mut Budget::default()).unwrap();
    assert!(f.curve_to_source_estimate_m >= 0.75);
    assert!(f.source_to_curve_estimate_m > 0.1);
    assert!(!f.within(Fit {
        curve_to_source_estimate_m: 0.1,
        source_to_curve_estimate_m: 0.1
    }));
}

#[test]
fn straight_fit_and_length_are_zero_and_exact() {
    let c = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let f = fit(&[c], &[c[0], c[3]], &mut Budget::default()).unwrap();
    assert_eq!(f.curve_to_source_estimate_m, 0.0);
    assert_eq!(f.source_to_curve_estimate_m, 0.0);
    assert_eq!(lengths(&[c])["chord_estimate_m"], 3.0);
}

#[test]
fn source_fit_budget_stops_without_a_partial_acceptance() {
    let c = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let mut budget = Budget {
        quality_work: WORK_LIMIT,
        ..Budget::default()
    };
    assert!(fit(&[c], &[c[0], c[3]], &mut budget).is_none());
    assert_eq!(budget.stop, Some("source_fit_work"));
}
