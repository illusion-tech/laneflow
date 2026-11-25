//! 路面坡缝两侧的来源水平切向。直线、贝塞尔只需要段端控制边，站位参数的正比例因子不影响朝向。
use super::{CurveSegment, NumericFreezeError, Point3, S, Scalar, Vector, ZERO};

type Point = [f64; 2];
type BoundedPoint = [Scalar; 2];

fn horizontal(point: Point3) -> BoundedPoint {
    [S(point.x), S(point.z)]
}

fn bounded_start(point: Point3, error: Vector) -> BoundedPoint {
    let with_error = |value, delta| {
        if delta == ZERO {
            S(value)
        } else {
            S(value) + delta
        }
    };
    [with_error(point.x, error[0]), with_error(point.z, error[2])]
}

pub(super) fn same_stored_heading(left: [f32; 9], right: [f32; 9]) -> bool {
    let [x, z, u, v] = [left[3], left[5], right[3], right[5]].map(f64::from);
    x * v == z * u && x * u + z * v > 0.0
}

pub(super) fn same_source_heading(
    previous: CurveSegment<'_>,
    previous_start_error: Vector,
    next: CurveSegment<'_>,
    next_start_error: Vector,
) -> Result<bool, NumericFreezeError> {
    let outgoing = match next {
        // 原生段入口按构造继承前段的水平切向，改变纵坡仍继承同一方向。
        CurveSegment::Native(_) => return Ok(true),
        CurveSegment::Line { start, end }
        | CurveSegment::CubicBezier {
            start,
            control_1: end,
            ..
        } => [bounded_start(start, next_start_error), horizontal(end)],
    };
    let incoming = match previous {
        CurveSegment::Line { start, end } => {
            [bounded_start(start, previous_start_error), horizontal(end)]
        }
        CurveSegment::CubicBezier { control_2, end, .. } => {
            [horizontal(control_2), horizontal(end)]
        }
        CurveSegment::Native(curve) => {
            let (_, tangent) = curve.vectors(1.0)?;
            [[ZERO; 2], [tangent[0], tangent[2]]]
        }
    };
    // 叉积、点积关于每个端点分量分别仿射。所有顶点精确共线且同向，才覆盖整个误差盒。
    Ok(control_edges(incoming)
        .all(|left| control_edges(outgoing).all(|right| cooriented(left, right))))
}

fn control_edges([start, end]: [BoundedPoint; 2]) -> impl Iterator<Item = [Point; 2]> {
    fn corners([x, z]: BoundedPoint) -> impl Iterator<Item = Point> {
        let ends = |c: Scalar| std::iter::once(c.lo).chain((c.hi != c.lo).then_some(c.hi));
        ends(x).flat_map(move |x| ends(z).map(move |z| [x, z]))
    }
    corners(start).flat_map(move |a| corners(end).map(move |b| [a, b]))
}

fn cooriented([a, b]: [Point; 2], [c, d]: [Point; 2]) -> bool {
    if ![a, b, c, d].into_iter().flatten().all(f64::is_finite) {
        return false;
    }
    let forward = (0..2).any(|i| (a[i] < b[i] && c[i] < d[i]) || (a[i] > b[i] && c[i] > d[i]));
    // 展开 (b-a) × (d-c)，保留来源坐标相减和相乘的所有有效位。
    forward
        && exact_zero_sum([
            (b[0], d[1]),
            (-b[0], c[1]),
            (-a[0], d[1]),
            (a[0], c[1]),
            (-b[1], d[0]),
            (b[1], c[0]),
            (a[1], d[0]),
            (-a[1], c[0]),
        ])
}

fn exact_zero_sum(products: [(f64, f64); 8]) -> bool {
    // 有限 binary64 = 带符号的至多 53 位整数 × 2^power，次正规数沿用最小指数。
    fn parts(value: f64) -> (i128, i32) {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let significand = (bits & ((1_u64 << 52) - 1)) | (u64::from(exponent != 0) << 52);
        let magnitude = i128::from(significand);
        (
            if value.is_sign_negative() {
                -magnitude
            } else {
                magnitude
            },
            exponent.max(1) - 1023 - 52,
        )
    }
    let mut terms = products.map(|(a, b)| {
        let (a, p) = parts(a);
        let (b, q) = parts(b);
        (p + q, a * b)
    });
    terms.sort_unstable_by_key(|&(power, _)| power);
    let mut sum = 0_i128;
    let mut unit = terms[0].0;
    for (power, coefficient) in terms {
        if coefficient == 0 {
            continue;
        }
        // 每项至多 106 位，八项之和至多 109 位。更高指数的项不能消掉最低位的余数。
        if sum != 0 {
            let shift = power.abs_diff(unit);
            if shift > sum.trailing_zeros() {
                return false;
            }
            sum >>= shift;
        }
        sum += coefficient;
        unit = power;
    }
    sum == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_forward_rays_keep_low_bits_across_scales() {
        let origin = [0.0; 2];
        let tiny = f64::from_bits(1);
        let a = [3.0 * tiny, -4.0 * tiny];
        let b = [1.5, -2.0];
        assert!(cooriented([origin, a], [origin, b]));
        assert!(!cooriented([origin, a], [b, origin]));
        assert!(!cooriented([origin, origin], [origin, b]));
        assert!(!cooriented(
            [origin, a],
            [origin, [3.0 * tiny, -5.0 * tiny]]
        ));
        assert!(!cooriented([[tiny, 0.0], b], [origin, b]));
    }

    #[test]
    fn uncertainty_must_preserve_the_direction_for_the_whole_box() {
        let start = Point3::try_new(-10.0, 0.0, 0.0).unwrap();
        let joint = Point3::try_new(0.0, 0.0, 0.0).unwrap();
        let end = Point3::try_new(5.0, 0.1, 0.0).unwrap();
        let first = CurveSegment::Line { start, end: joint };
        let next = CurveSegment::Line { start: joint, end };
        let uncertain = ZERO.widen(1e-8);
        assert!(same_source_heading(first, [uncertain, ZERO, ZERO], next, [ZERO; 3]).unwrap());
        assert!(!same_source_heading(first, [ZERO, ZERO, uncertain], next, [ZERO; 3]).unwrap());
    }
}
