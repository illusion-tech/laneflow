//! 在源站位行内用解析导数包围整个闭区间。证明细分不增加发布行。
use super::*;
use crate::road_editing::boundary::SurfaceRow;
use crate::road_editing::native_curve::{Scalar, Vector, exact_point, norm};

mod seam;

type Result<T> = std::result::Result<T, GeometryCompilationError>;
const S: fn(f64) -> Scalar = Scalar::exact;
const ZERO: Scalar = Scalar {
    value: 0.,
    lo: 0.,
    hi: 0.,
};

#[derive(Clone)]
struct Knot {
    station: f64,
    source: SourceLocation,
}

fn range(lo: f64, hi: f64) -> Scalar {
    Scalar {
        lo,
        hi,
        value: lo + (hi - lo) * 0.5,
    }
}
fn meters(n: u64) -> Scalar {
    let v = n as f64;
    let x = if n <= 1 << 53 {
        S(v)
    } else {
        range(v.next_down(), v.next_up())
    };
    x / S(1000.0)
}

struct Cell<'a> {
    curve: CurveSegment<'a>,
    start_point_error: Vector,
    row: &'a ReferenceStationRow,
    source_stations: [f64; 2],
    nodes: [(u64, u32); 2],
}
impl Cell<'_> {
    fn length(&self) -> Scalar {
        meters(self.nodes[1].0 - self.nodes[0].0)
    }

    /// 原线关于路面站位的零至四阶导数。自变量 x 是规范区间的无量纲坐标。
    fn derivatives(&self, x: Scalar) -> std::result::Result<[Vector; 5], NumericFreezeError> {
        let r = self.row;
        // 量化后的共同站点仍须准确指向原段的同一端面。直接把毫米站位
        // 代回原线会把某一侧真实纵坡延长到接缝另一侧。
        let [start, end] = self.source_stations.map(S);
        let span = end - start;
        let delta = start - S(r.cumulative_start_meters).widen(r.station_error_meters) + span * x;
        let rate = match self.curve {
            CurveSegment::Native(n) => S(1.) / n.law.road_length(),
            _ => {
                (S(r.parameter_end) - S(r.parameter_start))
                    / (S(r.cumulative_end_meters) - S(r.cumulative_start_meters))
            }
        };
        let t = S(r.parameter_start) + delta * rate;
        let mut d = [[ZERO; 3]; 5];
        match self.curve {
            CurveSegment::Native(n) => {
                let m = t.value.clamp(0., 1.);
                let du = (n.law.length * (t - S(m))).abs_max();
                let (pos, tangent) = n.vectors(m)?;
                let k = n.law.k0 + (n.law.k1 - n.law.k0) * t;
                let turn = (S(du) * S(k.abs_max().max(n.law.curvature(m).abs_max()))).hi;
                let rho = S(1.) / (S(1.) + n.law.grade.square()).sqrt();
                let a = k * rho;
                let b = (n.law.k1 - n.law.k0) / n.law.length * rho.square();
                let h = [tangent[0].widen(turn), ZERO, tangent[2].widen(turn)];
                let normal = [h[2], ZERO, -h[0]];
                d[0] = [
                    pos[0].widen(du),
                    pos[1] + n.law.grade * n.law.length * (t - S(m)),
                    pos[2].widen(du),
                ];
                for i in [0, 2] {
                    d[1][i] = rho * h[i];
                    d[2][i] = rho * a * normal[i];
                    d[3][i] = rho * (b * normal[i] - a.square() * h[i]);
                    d[4][i] = -rho * (S(3.) * a * b * h[i] + a.square() * a * normal[i]);
                }
                d[1][1] = n.law.grade * rho;
            }
            CurveSegment::Line { start, end } => {
                let p = exact_point(start);
                let end = exact_point(end);
                for i in 0..3 {
                    let p = p[i] + self.start_point_error[i];
                    let v = end[i] - p;
                    d[0][i] = p + v * t;
                    d[1][i] = v * rate;
                }
            }
            CurveSegment::CubicBezier {
                start,
                control_1,
                control_2,
                end,
            } => {
                let [p, q, r, z] = [start, control_1, control_2, end].map(exact_point);
                for i in 0..3 {
                    let p = p[i] + self.start_point_error[i];
                    let a = S(3.) * (q[i] - p);
                    let b = S(3.) * ((r[i] - q[i]) - (q[i] - p));
                    let c = (z[i] - r[i]) - S(2.) * (r[i] - q[i]) + (q[i] - p);
                    d[0][i] = ((c * t + b) * t + a) * t + p;
                    d[1][i] = ((S(3.) * c * t + S(2.) * b) * t + a) * rate;
                    d[2][i] = (S(6.) * c * t + S(2.) * b) * rate.square();
                    d[3][i] = S(6.) * c * rate.square() * rate;
                }
            }
        }
        // d 目前关于来源站位。发布导数关于规范站位，线性对应使第 k 阶
        // 多出 (来源站长 / 规范站长)^k，证明积分也使用同一规范自变量。
        let scale = span / self.length();
        let mut power = S(1.);
        for derivative in &mut d[1..] {
            power = power * scale;
            *derivative = derivative.map(|v| v * power);
        }
        Ok(d)
    }
}

type Polynomial = [[Scalar; 6]; 3];
fn polynomial(jets: [[f32; 9]; 2], h: Scalar) -> Polynomial {
    std::array::from_fn(|i| {
        let [p, q] = jets.map(|j| {
            [
                S(f64::from(j[i])),
                h * S(f64::from(j[i + 3])),
                h.square() * S(f64::from(j[i + 6])),
            ]
        });
        let [a, b, c] = [p[0], p[1], p[2] / S(2.)];
        let v = q[0] - (a + b + c);
        let w = q[1] - (b + S(2.) * c);
        let z = q[2] - S(2.) * c;
        [
            a,
            b,
            c,
            S(10.) * v - S(4.) * w + z / S(2.),
            -S(15.) * v + S(7.) * w - z,
            S(6.) * v - S(3.) * w + z / S(2.),
        ]
    })
}
fn derivative(poly: &Polynomial, x: Scalar, h: Scalar, order: usize) -> Vector {
    std::array::from_fn(|axis| {
        let mut acc = ZERO;
        for i in (order..=5).rev() {
            let factor = (0..order).fold(1, |n, j| n * (i - j));
            acc = acc * x + S(factor as f64) * poly[axis][i];
        }
        for _ in 0..order {
            acc = acc / h;
        }
        acc
    })
}
fn up_f32(x: f64) -> Option<f32> {
    let v = x as f32;
    let v = if f64::from(v) < x { v.next_up() } else { v };
    (x >= 0. && v.is_finite()).then_some(v)
}
fn jet(d: [Vector; 5]) -> std::result::Result<[f32; 9], NumericFreezeError> {
    let p = quantize_point(Point3::try_new(
        d[0][0].value,
        d[0][1].value,
        d[0][2].value,
    )?)?;
    let mut out = std::array::from_fn(|i| {
        let v = d[i / 3][i % 3].value as f32;
        if v == 0. { 0. } else { v }
    });
    out[..3].copy_from_slice(&[p.x, p.y, p.z]);
    if out.iter().any(|v| !v.is_finite()) {
        return Err(NumericFreezeKind::NonFinite.into());
    }
    Ok(out)
}

#[derive(Clone, Copy)]
struct Cone {
    axis: [Scalar; 2],
    min_parallel: f64,
    max_perpendicular: f64,
    max_speed: f64,
    max_normal_rate: f64,
}
impl Cone {
    fn observe(&mut self, velocity: Vector, acceleration: Vector) {
        let [a, b] = self.axis;
        self.min_parallel = self
            .min_parallel
            .min((a * velocity[0] + b * velocity[2]).lo);
        self.max_perpendicular = self
            .max_perpendicular
            .max((a * velocity[2] - b * velocity[0]).abs_max());
        self.max_speed = self
            .max_speed
            .max(norm([velocity[0], ZERO, velocity[2]]).hi);
        // 只有垂直于水平切向的加速度会改变法向。沿直线变速对本项的贡献为零。
        let speed_squared = velocity[0].square() + velocity[2].square();
        let turn_rate = if speed_squared.lo > 0.0 {
            ((velocity[0] * acceleration[2] - velocity[2] * acceleration[0]) / speed_squared)
                .abs_max()
        } else {
            f64::INFINITY
        };
        self.max_normal_rate = self.max_normal_rate.max(turn_rate);
    }
    fn separates(&self, width: f64) -> bool {
        // 对任意 s<t，将两截面的位移投到 s 处切向。中心线投影下界须严格
        // 大于宽度乘单位法向的 Lipschitz 界，同时证明非零速度、正则性和全域单值。
        let p = S(self.min_parallel);
        let directed = (p.square() - S(self.max_perpendicular).square()) / S(self.max_speed);
        if self.min_parallel <= 0. {
            return false;
        }
        let normal_rate = S(self.max_normal_rate);
        let needed = S(width) * normal_rate;
        // 若两截面相交，沿固定轴的中心线净前进至多等于两倍横宽。
        // 因此只需比较这一段站距内的切向；单位向量的点积为 1-|T1-T2|²/2。
        let nearby = S(2.) * S(width) / p;
        let local = p * (S(1.) - (normal_rate * nearby).square() / S(2.));
        directed.lo.max(local.lo) > needed.hi
    }
}
fn proof(cell: &Cell<'_>, jets: [[f32; 9]; 2], cones: &mut [Cone; 3]) -> Option<[f32; 5]> {
    let h = cell.length();
    let poly = polynomial(jets, h);
    let mut errors = [0_f64; 5];
    // 闭子区间上先直接包围四阶差。由左端实际差逐阶积分，避免把两个高度相关
    // 的绝对位置区间直接相减。每片独立起算，保证前缀误差不逐行膨胀。
    const PARTS: usize = 8;
    for i in 0..PARTS {
        let a = i as f64 / PARTS as f64;
        let b = (i + 1) as f64 / PARTS as f64;
        let x = range(a, b);
        let at = cell.derivatives(S(a)).ok()?;
        let all = cell.derivatives(x).ok()?;
        let mut diff = ZERO;
        for k in (0..=4).rev() {
            let value = if k == 4 { all[k] } else { at[k] };
            let base = derivative(&poly, if k == 4 { x } else { S(a) }, h, k);
            diff = norm(std::array::from_fn(|j| value[j] - base[j])) + diff * h / S(PARTS as f64);
            errors[k] = errors[k].max(diff.finite().ok()?.hi);
        }
        let velocity = derivative(&poly, x, h, 1);
        let acceleration = derivative(&poly, x, h, 2);
        for cone in &mut *cones {
            cone.observe(velocity, acceleration);
            cone.observe(all[1], all[2]);
        }
    }
    let mut result = [0.; 5];
    for (out, e) in result.iter_mut().zip(errors) {
        *out = up_f32(e)?;
    }
    Some(result)
}

pub(super) fn build(
    plan: &ResolvedCorridorPlan<'_>,
    alignment: &CompiledAlignmentEntry<'_>,
    domain: [f64; 2],
    extents: &[ExtentRow],
    width: f64,
    scratch: &mut GeometryScratchBudget,
) -> Result<Box<[SurfaceRow]>> {
    let g = plan.corridor.authoring_geometry.as_ref().expect("resolved");
    let coverage = g.station_mapping_request.as_ref().expect("complete");
    let program = &alignment.declaration.reference_line;
    let mut count = extents.len().saturating_mul(2).saturating_add(2);
    for e in &plan.elements {
        if let ResolvedCorridorElement::RoadSection(s) = e {
            for l in &s.lanes {
                count = count.saturating_add(
                    l.authoring_geometry
                        .as_ref()
                        .expect("resolved")
                        .interior_cuts
                        .len(),
                );
            }
        }
    }
    scratch.reserve::<Knot>(count)?;
    let mut knots = Vec::with_capacity(count);
    for (i, value) in domain.into_iter().enumerate() {
        let id = if i == 0 {
            2
        } else if matches!(g.end_station, AuthoringStationEnd::AlignmentEnd) {
            3
        } else {
            4
        };
        knots.push(Knot {
            station: value,
            source: plan
                .corridor
                .header
                .span
                .with_property([field(Table::RoadCorridor, id)]),
        });
    }
    for e in extents {
        for (i, station) in e.declaration.ends.iter().enumerate() {
            if let BoundaryStation::Fixed(value) = station {
                knots.push(Knot {
                    station: *value,
                    source: e.declaration.station_source(i),
                });
            }
        }
    }
    for e in &plan.elements {
        if let ResolvedCorridorElement::RoadSection(s) = e {
            for l in &s.lanes {
                for (station, source) in &l
                    .authoring_geometry
                    .as_ref()
                    .expect("resolved")
                    .interior_cuts
                {
                    knots.push(Knot {
                        station: *station,
                        source: source.clone(),
                    });
                }
            }
        }
    }
    knots.sort_unstable_by(|a, b| {
        a.station
            .total_cmp(&b.station)
            .then(a.source.cmp(&b.source))
    });
    let rows = &alignment.reference.station_rows;
    let mut capacity = knots.len();
    for row in rows {
        let a = row.cumulative_start_meters.max(domain[0]);
        let b = row.cumulative_end_meters.min(domain[1]);
        if a >= b {
            continue;
        }
        capacity = capacity.saturating_add(1);
        if matches!(&program.segments[row.segment_ordinal as usize].geometry,
            AuthoringCurveSegmentGeometry::Native{prepared:Some(n),..} if n.law.k0.value != n.law.k1.value)
        {
            let first = knots.partition_point(|k| k.station <= a);
            let last = knots.partition_point(|k| k.station < b);
            capacity = capacity.saturating_add(1 + last - first);
        }
    }
    retain::<SurfaceRow>(scratch, capacity)?;
    let mut result: Vec<SurfaceRow> = Vec::with_capacity(capacity);
    let station_node = |s: f64, r: &ReferenceStationRow| {
        crate::road_editing::station_proof::node(
            domain[0],
            S(s).widen(r.station_error_meters),
            Some(s),
        )
        .map_err(|_| not_proven(coverage))
    };
    let mut pos = point3(program.start, CurvePointRole::Start).map_err(|_| not_proven(coverage))?;
    let mut start_error = [ZERO; 3];
    let mut row_index = 0;
    let mut cone = None;
    let mut previous_segment = None;
    for (si, seg) in program.segments.iter().enumerate() {
        let fail = || not_proven(&seg.span);
        let (curve, end) = source_segment(pos, seg).map_err(|_| fail())?;
        let mut incoming = previous_segment.replace((curve, start_error));
        while let Some(row) = rows
            .get(row_index)
            .filter(|r| r.segment_ordinal as usize == si)
        {
            row_index += 1;
            let mut a = row.cumulative_start_meters.max(domain[0]);
            let end = row.cumulative_end_meters.min(domain[1]);
            let mut refinement = None;
            while a < end {
                let k = knots.partition_point(|k| k.station <= a);
                let mandatory_end = knots.get(k).map_or(end, |k| end.min(k.station));
                let b = refinement.filter(|mid| *mid > a).unwrap_or(mandatory_end);
                let nodes = [station_node(a, row)?, station_node(b, row)?];
                if nodes[0].0 >= nodes[1].0 {
                    return Err(fail());
                }
                let cell = Cell {
                    curve,
                    start_point_error: start_error,
                    row,
                    source_stations: [a, b],
                    nodes,
                };
                // 同一原段内的分行共享源切向。跨原段时先在真实的段端点核对水平朝向。
                let source_seam = incoming.filter(|_| !result.is_empty() && !seg.smooth_entry);
                if let Some((previous, previous_error)) = source_seam
                    && !seam::same_source_heading(previous, previous_error, curve, start_error)
                        .map_err(|_| fail())?
                {
                    return Err(fail());
                }
                let mut jets = [
                    jet(cell.derivatives(S(0.)).map_err(|_| fail())?).map_err(|_| fail())?,
                    jet(cell.derivatives(S(1.)).map_err(|_| fail())?).map_err(|_| fail())?,
                ];
                if let Some(last) = result.last() {
                    let last_jet = last.jets[1];
                    if last.stations[1].0 != nodes[0].0 {
                        return Err(fail());
                    }
                    jets[0][..3].copy_from_slice(&last_jet[..3]);
                    // 真实坡缝若已有共同规范法向，两侧可保留各自水平速率。proof 用最终 jets 包住调整量。
                    if source_seam.is_none() || !seam::same_stored_heading(last_jet, jets[0]) {
                        jets[0][3] = last_jet[3];
                        jets[0][5] = last_jet[5];
                    }
                }
                let cone = cone.get_or_insert_with(|| {
                    let [x, z] = [jets[0][3], jets[0][5]].map(|v| S(f64::from(v)));
                    [[x, z], [x + z, z - x], [x - z, z + x]].map(|[x, z]| {
                        let h = norm([x, ZERO, z]);
                        Cone {
                            axis: [x / h, z / h],
                            min_parallel: f64::INFINITY,
                            max_perpendicular: 0.,
                            max_speed: 0.,
                            max_normal_rate: 0.,
                        }
                    })
                });
                let mut trial = *cone;
                let errors = proof(&cell, jets, &mut trial).ok_or_else(fail)?;
                // 转向验算中 1e-6 m⁻² 对应 19 m/s、2.6 m 轴距下约 0.00005 rad/s。
                // 曲率变化段的三阶证据太宽时补一个共同路面锚点。每个必选区间最多补一个；
                // 最终仍发布真实误差，查询使用车型自己的限值。
                if refinement.is_none()
                    && errors[3] > 1e-6
                    && matches!(curve, CurveSegment::Native(n) if n.law.k0.value!=n.law.k1.value)
                {
                    let mid = a + (b - a) * 0.5;
                    let node = station_node(mid, row)?;
                    if nodes[0].0 < node.0 && node.0 < nodes[1].0 {
                        refinement = Some(mid);
                        continue;
                    }
                }
                *cone = trial;
                incoming = None;
                if b == mandatory_end {
                    refinement = None;
                }
                let mut endpoint_sources = [a, b].map(|n| {
                    let first = knots.partition_point(|k| k.station < n);
                    &knots[first..knots.partition_point(|k| k.station <= n)]
                });
                let n = 3 + endpoint_sources.iter().map(|k| k.len()).sum::<usize>();
                retain::<SourceLocation>(scratch, n)?;
                let mut sources = Vec::with_capacity(n);
                sources.extend([
                    g.road_alignment_key_span.clone(),
                    coverage.clone(),
                    alignment.declaration.canonical_frame.span.clone(),
                ]);
                for list in &mut endpoint_sources {
                    sources.extend(list.iter().map(|k| k.source.clone()));
                }
                sources.sort_unstable();
                sources.dedup();
                result.push(SurfaceRow {
                    stations: nodes,
                    jets,
                    errors,
                    source: RowSource {
                        primary: seg.span.clone(),
                        contributing: sources.into_boxed_slice(),
                    },
                });
                a = b;
            }
        }
        pos = end;
        start_error = if let CurveSegment::Native(n) = curve {
            let (p, _) = n.vectors(1.).map_err(|_| fail())?;
            p.map(|s| s - S(s.value))
        } else {
            [ZERO; 3]
        };
    }
    if !cone.is_some_and(|cones| cones.iter().any(|c| c.separates(width))) {
        return Err(not_proven(coverage));
    }
    scratch.release::<Knot>(count);
    Ok(result.into_boxed_slice())
}
