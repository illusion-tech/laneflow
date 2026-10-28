//! 有限 f64 输入上的外向舍入包围；只用于固定平面 cubic 的必要条件排除。

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub(super) lo: f64,
    pub(super) hi: f64,
}

impl Interval {
    pub(super) fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }

    pub(super) fn add(self, b: Self) -> Self {
        Self {
            lo: (self.lo + b.lo).next_down(),
            hi: (self.hi + b.hi).next_up(),
        }
    }

    pub(super) fn sub(self, b: Self) -> Self {
        Self {
            lo: (self.lo - b.hi).next_down(),
            hi: (self.hi - b.lo).next_up(),
        }
    }

    pub(super) fn mul(self, b: Self) -> Self {
        let p = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        Self {
            lo: p.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            hi: p.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        }
    }

    pub(super) fn max_abs(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }

    pub(super) fn min_abs(self) -> f64 {
        if self.lo <= 0.0 && self.hi >= 0.0 {
            0.0
        } else {
            self.lo.abs().min(self.hi.abs())
        }
    }
}

pub(super) type Box2 = [Interval; 2];

pub(super) fn point(p: [f32; 3]) -> Box2 {
    [
        Interval::point(f64::from(p[0])),
        Interval::point(f64::from(p[2])),
    ]
}

pub(super) fn difference(a: Box2, b: Box2) -> Box2 {
    [a[0].sub(b[0]), a[1].sub(b[1])]
}

pub(super) fn short(v: Box2) -> bool {
    let x = Interval::point(v[0].max_abs());
    let y = Interval::point(v[1].max_abs());
    // 比真实 0.1 m 规则更宽松，留下远大于 f32 hypot 舍入误差的间隔。
    x.mul(x).add(y.mul(y)).hi < 0.099_999_f64.powi(2)
}

pub(super) fn angle_excluded(a: Box2, b: Box2, tangent_upper: f64) -> bool {
    let dot = a[0].mul(b[0]).add(a[1].mul(b[1]));
    let cross = a[0].mul(b[1]).sub(a[1].mul(b[0]));
    dot.hi < 0.0
        || cross.min_abs()
            > Interval::point(tangent_upper)
                .mul(Interval::point(dot.hi.max(0.0)))
                .hi
}

/// 包围数学 cubic 与当前 `cubic_at` 的浮点求值，量化后仍为超集。
pub(super) fn curve(c: [[f64; 3]; 4], t: Interval) -> Box2 {
    let mid = (t.lo + t.hi) * 0.5;
    let radius = (mid - t.lo).max(t.hi - mid).next_up();
    [0, 2].map(|axis| {
        let controls = c.map(|p| Interval::point(p[axis]));
        let mut values = controls;
        let u = Interval::point(mid);
        let v = Interval::point(1.0).sub(u);
        for width in (1..4).rev() {
            for i in 0..width {
                values[i] = values[i].mul(v).add(values[i + 1].mul(u));
            }
        }
        let speed = (0..3)
            .map(|i| {
                controls[i + 1]
                    .sub(controls[i])
                    .mul(Interval::point(3.0))
                    .max_abs()
            })
            .fold(0.0, f64::max);
        let scale = controls
            .into_iter()
            .map(Interval::max_abs)
            .fold(1.0, f64::max);
        // 正权重的直接 Bernstein 求值不足 32 次基本运算；128 epsilon 留出余量。
        let roundoff = Interval::point(scale).mul(Interval::point(128.0 * f64::EPSILON));
        let spread = Interval::point(speed)
            .mul(Interval::point(radius))
            .add(roundoff)
            .hi;
        let bounds = values[0].add(Interval {
            lo: -spread,
            hi: spread,
        });
        Interval {
            lo: f64::from(bounds.lo as f32),
            hi: f64::from(bounds.hi as f32),
        }
    })
}

#[test]
fn outward_curve_contains_direct_evaluation_and_rounding_boundaries() {
    let c = [
        [-1_084.34, 0.0, 1_709.47],
        [-1_084.897, 0.0, 1_709.586],
        [-1_085.479, 0.0, 1_709.462],
        [-1_086.048, 0.0, 1_709.459],
    ];
    for i in 0..1_024 {
        let t = Interval {
            lo: i as f64 / 1_024.0,
            hi: (i + 1) as f64 / 1_024.0,
        };
        let bounds = curve(c, t);
        for j in 0..=16 {
            let u = t.lo + (t.hi - t.lo) * f64::from(j) / 16.0;
            let p = super::quantize_vec3(super::cubic_at(c[0], c[1], c[2], c[3], u));
            for (k, axis) in [0, 2].into_iter().enumerate() {
                assert!(bounds[k].lo <= f64::from(p[axis]) && f64::from(p[axis]) <= bounds[k].hi);
            }
        }
    }
    assert!(angle_excluded(
        point([1.0, 0.0, 0.0]),
        point([1.0, 0.0, 0.1]),
        0.0341
    ));
    assert!(!angle_excluded(
        point([1.0, 0.0, 0.0]),
        point([1.0, 0.0, 0.02]),
        0.0341
    ));
}

#[test]
fn narrow_enclosures_cover_machine_evaluation_at_extrema_and_float_boundaries() {
    let boundary = (f64::from(1_709.5_f32) + f64::from(1_709.5_f32.next_up())) * 0.5;
    let cases = [
        [
            [32_000.0, 0.0, -32_000.0],
            [-32_000.0, 0.0, 32_000.0],
            [32_000.0, 0.0, -32_000.0],
            [-32_000.0, 0.0, 32_000.0],
        ],
        [[boundary, 0.0, 0.0]; 4],
        [
            [0.0, 0.0, 0.0],
            [f64::MIN_POSITIVE, 0.0, 0.0],
            [-f64::MIN_POSITIVE, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ],
    ];
    for c in cases {
        for power in [10, 24, 40, 48] {
            let width = 2.0_f64.powi(-power);
            for center in [0.0, 0.001, 0.25, 0.5, 0.726_552_963_256_836, 0.999, 1.0] {
                let t = Interval {
                    lo: (center - width).max(0.0),
                    hi: (center + width).min(1.0),
                };
                let b = curve(c, t);
                for u in [
                    t.lo,
                    t.lo.next_up(),
                    (t.lo + t.hi) * 0.5,
                    t.hi.next_down(),
                    t.hi,
                ] {
                    let q = super::quantize_vec3(super::at(c, u));
                    for (k, axis) in [0, 2].into_iter().enumerate() {
                        assert!(b[k].lo <= f64::from(q[axis]) && f64::from(q[axis]) <= b[k].hi);
                    }
                }
            }
        }
    }
}
