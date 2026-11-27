use super::*;

fn rail(key: &str, station: f64, length: f64, offset: f64, bottom: f64) -> BoundaryExtentInput {
    extent(
        key,
        BoundaryStation::Fixed(station),
        BoundaryStation::Fixed(station + length),
        BoundaryPayloadInput::Guardrail {
            anchor_role: 1,
            lateral_offset_meters: offset,
            horizontal_length_meters: length,
            radius_meters: 0.05,
            bottom_height_meters: bottom,
            top_height_meters: bottom + 1.,
            parameter_error_meters: 0.,
        },
    )
}

#[test]
fn two_openings_have_local_bounds_only_while_the_body_is_alongside() {
    let road = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 52.))]).unwrap();
    let curb_end = CurbEndInput {
        left_offset_meters: -0.2,
        right_offset_meters: -0.3,
        bottom_height_meters: 0.,
        top_height_meters: 1.,
    };
    let boundaries = vec![
        rail("post-left", 22., 1., 0., 0.),
        rail("post-right", 22., 1., 2.1, 0.),
        rail("overhead", 12., 21., 1.05, 3.),
        extent(
            "curb",
            BoundaryStation::Fixed(31.),
            BoundaryStation::Fixed(33.),
            BoundaryPayloadInput::Curb {
                low: curb_end,
                high: curb_end,
                parameter_error_meters: 0.,
            },
        ),
    ];
    let n = load_network(
        &with_boundaries(road, 2.1, boundaries),
        SpatialBuildOption::Omit,
    );
    let q = corridor_query(&n, [10_000_000_000, 38_000_000_000], 0.);
    assert!(n.vehicle_geometry(&q, LIMITS, BODY, BUDGET).feasible());
    let probe = |limit| n.vehicle_side_clearances(&q, BODY, 1_000_000_000, limit);
    let full = probe(BUDGET.sweep);
    let spans = full
        .segments
        .as_ref()
        .unwrap_or_else(|_| panic!("{full:?}"));
    assert_eq!(spans.len(), 28);
    for (i, row) in spans.iter().enumerate() {
        let at = i as u64 * 1_000_000_000;
        assert_eq!(row.range_nm, [at, at + 1_000_000_000]);
    }
    assert!(spans[..7].iter().all(|s| s.clearance.is_none()));
    let narrow = spans[11].clearance.unwrap();
    assert!(matches!(
        narrow.boundary.key,
        Some("post-left" | "post-right")
    ));
    assert!(
        (49_990..=50_000).contains(&narrow.lower_micrometres()),
        "{narrow:?}"
    );
    assert!(spans[14..16].iter().all(|s| s.clearance.is_none()));
    let curb = spans[20].clearance.unwrap();
    assert_eq!(curb.boundary.key, Some("curb"));
    assert!(
        (299_990..=300_000).contains(&curb.lower_micrometres()),
        "{curb:?}"
    );
    assert!(spans[25..].iter().all(|s| s.clearance.is_none()));
    assert_eq!(probe(full.work), full);
    let short = probe(full.work - 1);
    assert_eq!(short.segments, Err(Failure::WorkLimit));
    assert_eq!(short.work, full.work - 1);
    assert!(short.unproven_range_nm.is_some());
    let invalid = n.vehicle_side_clearances(&q, BODY, 0, BUDGET.sweep);
    assert_eq!(invalid.segments, Err(Failure::InvalidInput));
}

#[test]
fn oblique_curb_keeps_the_normal_clearance_when_the_world_boxes_are_disjoint() {
    let road = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(6.8, 0., 40.))]).unwrap();
    let end = CurbEndInput {
        left_offset_meters: -0.2,
        right_offset_meters: -0.3,
        bottom_height_meters: 0.,
        top_height_meters: 1.,
    };
    let curb = extent(
        "diagonal-curb",
        BoundaryStation::Fixed(20.),
        BoundaryStation::Fixed(21.),
        BoundaryPayloadInput::Curb {
            low: end,
            high: end,
            parameter_error_meters: 0.,
        },
    );
    let n = load_network(
        &with_boundaries(road, 2.1, vec![curb]),
        SpatialBuildOption::Omit,
    );
    let q = corridor_query(&n, [20_000_000_000, 20_010_000_000], 0.);
    assert!(n.vehicle_geometry(&q, LIMITS, BODY, BUDGET).feasible());
    let report = n.vehicle_side_clearances(&q, BODY, 10_000_000, BUDGET.sweep);
    let spans = report.segments.unwrap();
    let clearance = spans[0].clearance.unwrap();
    assert_eq!(clearance.boundary.key, Some("diagonal-curb"));
    // 路缘与车身侧面平行且纵向相交。真实水平间距为 2.1/2 + 0.2 − 0.95 = 0.30 米。
    assert!(
        (0.295..=0.300_001).contains(&clearance.lower_m),
        "{clearance:?}"
    );

    // 后轴已越过端点至少 1 米，车尾最初即离开端点 0.25 米。
    let beyond = corridor_query(&n, [22_000_000_000, 22_400_000_000], 0.);
    let after = n.vehicle_side_clearances(&beyond, BODY, 400_000_000, BUDGET.sweep);
    assert!(after.segments.unwrap()[0].clearance.is_none());
}

#[test]
fn a_bend_retains_a_side_rail_until_the_rear_overhang_moves_beyond_its_cap() {
    let curve = Program::try_new_with_tangent(
        p(0., 0., 0.),
        vec![Segment::circular_arc(28., 35., 0.).unwrap()],
        Some([0., 1.]),
    )
    .unwrap();
    let marking = extent(
        "mark",
        BoundaryStation::CorridorEndpoint,
        BoundaryStation::CorridorEndpoint,
        BoundaryPayloadInput::CrossingRule {
            directions: 3,
            effect: 1,
        },
    );
    let n = load_network(
        &with_boundaries(curve, 4., vec![rail("bend", 12., 1., 0., 0.), marking]),
        SpatialBuildOption::Omit,
    );
    let q = corridor_query(&n, [14_000_000_000, 17_500_000_000], 0.);
    let extended = BodyEnvelope {
        rear_overhang_m: 3.,
        ..BODY
    };
    let check = |b| {
        n.vehicle_side_clearances(&q, b, 500_000_000, BUDGET.sweep)
            .segments
            .unwrap()
    };
    let admission = n.vehicle_geometry(&q, LIMITS, extended, BUDGET);
    assert!(admission.feasible(), "{admission:?}");
    let short = check(BODY);
    let long = check(extended);
    // 后轴已越过 13 米端帽；长后悬在弯中转向时仍位于它的前向投影内。
    assert!(short[2].clearance.is_none());
    let still_alongside = long[2].clearance.unwrap();
    assert_eq!(still_alongside.boundary.key, Some("bend"));
    // 用来源圆弧在后轴切向、法向坐标中直接定位 1 米护栏末端。
    // 车身右侧与端帽同纵坐标处的一对实点给出真实最小距离的上界。
    let (sin, cos) = ((12.0_f64 - 15.25) / 35.).sin_cos();
    let cap_forward = 37. * sin + cos;
    let cap_left = 35. - 37. * cos + sin;
    assert!(cap_forward < -BODY.rear_overhang_m && cap_forward > -extended.rear_overhang_m);
    let point_gap = -cap_left - extended.half_width_m - 0.05;
    assert!(still_alongside.lower_m > 0. && still_alongside.lower_m <= point_gap + 0.001);
    assert!(long.last().unwrap().clearance.is_none());
}
