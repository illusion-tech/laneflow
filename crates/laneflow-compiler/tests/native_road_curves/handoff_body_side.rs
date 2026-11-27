use super::*;
use laneflow_static_network::{
    BoundaryOwner,
    vehicle::{HandoffBodySide, HandoffQuery, HandoffWindow, SWEEP_WORK_LIMIT},
};

const M: u64 = 1_000_000_000;

#[test]
fn curved_seam_can_cut_the_body_even_with_every_corner_on_the_target_side() {
    let radius: f64 = 64.;
    let mut segments = vec![Segment::line(p(0., 0., 20.))];
    segments.extend((0..5).map(|_| Segment::circular_arc(10., radius, 0.).unwrap()));
    let circle = Program::try_new(p(0., 0., 0.), segments).unwrap();
    let artifact = road(circle, [([3.5; 2], true); 2]);
    let n = load_network(&artifact, SpatialBuildOption::Omit);
    let start = 45 * M + 123_000_000;
    let (q, w) = inputs(&n, [start, start + 18 * M]);
    let wide = BodyEnvelope {
        front_from_rear_m: 20.,
        rear_overhang_m: 20.,
        ..BODY
    };
    // 起始后轴在半径 64 m 参考车道的中心，车体沿圆的切线。共边在半径
    // 65.75 m 处。前、后端的最内角都在共边外，但车身纵向中截面全在共边内。
    let seam_radius = radius + 1.75;
    assert!((radius - wide.half_width_m).hypot(wide.front_from_rear_m) > seam_radius);
    assert!(radius + wide.half_width_m < seam_radius);
    let r = n.handoff_body_side(&q, w, wide, [start; 2], SWEEP_WORK_LIMIT);
    assert_eq!(r.side, Ok(HandoffBodySide::Straddling), "{r:?}");
    let clipped = HandoffQuery {
        station_window_nm: [40 * M, 65 * M],
        ..q
    };
    assert_eq!(
        n.handoff_body_side(&clipped, w, wide, [start; 2], SWEEP_WORK_LIMIT)
            .side,
        Err(Failure::Projection)
    );
}

fn inputs(n: &SharedNetworkRevision, rear: [u64; 2]) -> (HandoffQuery, HandoffWindow) {
    let q = corridor_query(n, rear, 0.);
    let r = n.traffic().relations();
    let laneflow_static_network::CorridorElement::RoadSection(section) =
        r.corridor_elements(q.corridor).unwrap()[0]
    else {
        unreachable!()
    };
    let lanes = r.section_lanes(section).unwrap();
    let window = HandoffWindow {
        edges: [0, 1].map(|i| r.authoring_edge_chain(lanes[i]).unwrap()[0]),
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: q.station_window_nm,
    };
    (
        HandoffQuery {
            revision: q.revision,
            corridor: q.corridor,
            frame: q.frame,
            station_window_nm: q.station_window_nm,
            axles: AXLES,
            rear_stations_nm: rear,
        },
        window,
    )
}

#[test]
fn body_side_at_handoff_endpoints_is_invariant_under_horizontal_rotation() {
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    for heading in [[0., 1.], [diagonal, diagonal]] {
        for (direction, rear) in [
            (RoadEditingLaneDirection::Forward, [10 * M, 40 * M]),
            (RoadEditingLaneDirection::Backward, [40 * M, 10 * M]),
        ] {
            let artifact = straight_road(heading, direction);
            let network = load_network(&artifact, SpatialBuildOption::Omit);
            let (q, window) = inputs(&network, rear);
            for edges in [window.edges, [window.edges[1], window.edges[0]]] {
                let w = HandoffWindow { edges, ..window };
                // 两端五次横移的切向都沿道路。两车道各宽 3.5 m，
                // 半车宽 0.95 m，起止姿态的完整车身距共边均有 0.80 m。
                for (station, expected) in rear
                    .into_iter()
                    .zip([HandoffBodySide::SourceOnly, HandoffBodySide::TargetOnly])
                {
                    let query = |limit| network.handoff_body_side(&q, w, BODY, [station; 2], limit);
                    let full = query(SWEEP_WORK_LIMIT);
                    assert_eq!(
                        full.side,
                        Ok(expected),
                        "{heading:?}, {direction:?}, {edges:?}"
                    );
                    assert_eq!(query(full.work), full);
                    assert_eq!(query(full.work - 1).side, Err(Failure::WorkLimit));
                }
            }
        }
    }
}

fn straight_road(
    [x, z]: [f64; 2],
    direction: RoadEditingLaneDirection,
) -> PortablePublicationCandidate {
    let at = |s| p(x * s, 0., z * s);
    let program =
        Program::try_new(at(0.), vec![Segment::line(at(25.)), Segment::line(at(50.))]).unwrap();
    let source = save_lane_declarations(
        program,
        &[],
        (
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
        ),
        [direction; 2],
        CrossSectionCoverage::Complete,
        0,
        SectionFixture::default(),
    );
    publish(&compile_saved(&source).unwrap(), "handoff-body-side")
}

#[test]
fn full_body_straddles_after_front_handoff_and_then_clears_in_both_directions() {
    for (direction, rear) in [
        (RoadEditingLaneDirection::Forward, [10 * M, 40 * M]),
        (RoadEditingLaneDirection::Backward, [40 * M, 10 * M]),
    ] {
        let artifact = straight_road([0., 1.], direction);
        let reports = [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ]
        .map(|option| {
            let n = load_network(&artifact, option);
            let (q, window) = inputs(&n, rear);
            let station = |offset| {
                if rear[0] < rear[1] {
                    rear[0] + offset
                } else {
                    rear[0] - offset
                }
            };
            let mut reports = Vec::new();
            for edges in [window.edges, [window.edges[1], window.edges[0]]] {
                let w = HandoffWindow { edges, ..window };
                // 30 m 五次横移的前保险杠已在后轴前进 11.8612 m 时过缝。13 m
                // 时两侧仍各有车身；25 m 附近整个车身已在目标侧。
                for (offset, side) in [
                    (5 * M, HandoffBodySide::SourceOnly),
                    (13 * M, HandoffBodySide::Straddling),
                    (15 * M, HandoffBodySide::Straddling),
                    (25 * M, HandoffBodySide::TargetOnly),
                ] {
                    let s = station(offset);
                    let range = [s - 1_000_000, s + 1_000_000];
                    let full = n.handoff_body_side(&q, w, BODY, range, SWEEP_WORK_LIMIT);
                    assert_eq!(full.side, Ok(side), "{offset} {full:?}");
                    assert_eq!(n.handoff_body_side(&q, w, BODY, range, full.work), full);
                    assert_eq!(
                        n.handoff_body_side(&q, w, BODY, range, full.work - 1).side,
                        Err(Failure::WorkLimit)
                    );
                    reports.push(full);
                }
                let tangent = BodyEnvelope {
                    half_width_m: 1.75,
                    ..BODY
                };
                assert_eq!(
                    n.handoff_body_side(&q, w, tangent, [rear[0]; 2], SWEEP_WORK_LIMIT)
                        .side,
                    Err(Failure::Boundary)
                );
                assert!(
                    n.handoff_body_side(&q, w, BODY, [12 * M, 38 * M], SWEEP_WORK_LIMIT)
                        .side
                        .is_err()
                );
                for range in [[20 * M, 19 * M], [0; 2]] {
                    assert_eq!(
                        n.handoff_body_side(&q, w, BODY, range, SWEEP_WORK_LIMIT)
                            .side,
                        Err(Failure::InvalidInput)
                    );
                }
            }
            reports
        });
        assert_eq!(reports[0], reports[1]);
    }
}
