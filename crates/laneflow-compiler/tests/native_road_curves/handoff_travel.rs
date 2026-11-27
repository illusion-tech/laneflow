use super::*;
use laneflow_static_contract::{CanonicalFrameOrdinal, RoadCorridorOrdinal};
use laneflow_static_network::{
    BoundaryOwner, CorridorElement,
    vehicle::{
        AxleDimensions, Failure, HandoffQuery, HandoffTravelBounds, HandoffTravelTarget,
        HandoffWindow, TRACK_WORK_LIMIT,
    },
};

const NM: u64 = 1_000_000_000;

// 坡面的 3D 纵向弧长为 s，横向五次位移为 3.5 f(s/30)。
// 两方向正交，三维速率为 sqrt(1 + (3.5 f'/30)^2)。独立地在速率单调的
// 前后半段用上下矩形积分。返回的双边界包括 f64 求和舍入余量。
fn oracle(distance_along_corridor: f64) -> [f64; 2] {
    let speed = |s: f64| {
        let u = s / 30.;
        (3.5 * (u * (1. - u)).powi(2)).hypot(1.)
    };
    let mut bounds = [0., 0.];
    for s in [
        [0., distance_along_corridor.min(15.)],
        [15., distance_along_corridor],
    ] {
        if s[0] >= s[1] {
            continue;
        }
        let count = 32768;
        let width = (s[1] - s[0]) / f64::from(count);
        let mut left = speed(s[0]);
        for i in 1..=count {
            let right = speed(s[0] + f64::from(i) * width);
            bounds[0] += width * left.min(right);
            bounds[1] += width * left.max(right);
            left = right;
        }
    }
    [bounds[0] - 1e-9, bounds[1] + 1e-9]
}

#[test]
fn graded_quintic_uses_rear_axle_arc_length_in_both_directions() {
    for (direction, start, end) in [
        (RoadEditingLaneDirection::Forward, 20 * NM, 50 * NM),
        (RoadEditingLaneDirection::Backward, 50 * NM, 20 * NM),
    ] {
        let slope = Program::try_new(
            p(0., 0., 0.),
            vec![
                Segment::line(p(0., 4., 39.9)),
                Segment::line(p(0., 8., 79.8)),
            ],
        )
        .unwrap();
        let source = save_lane_declarations(
            slope,
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
        let artifact = publish(&compile_saved(&source).unwrap(), "handoff-travel");
        let mut previous = None;
        for spatial in [
            SpatialBuildOption::Omit,
            SpatialBuildOption::RetainAvailable,
        ] {
            let n = load_network(&artifact, spatial);
            let corridor = RoadCorridorOrdinal::from_raw(0);
            let r = n.traffic().relations();
            let CorridorElement::RoadSection(section) = r.corridor_elements(corridor).unwrap()[0]
            else {
                unreachable!()
            };
            let lanes = r.section_lanes(section).unwrap();
            let q = HandoffQuery {
                revision: n.network_revision(),
                corridor,
                frame: CanonicalFrameOrdinal::from_raw(0),
                station_window_nm: [0, 70 * NM],
                rear_stations_nm: [start, end],
                axles: AxleDimensions {
                    wheelbase_m: 2.6,
                    half_track_m: 0.8,
                    reference_height_m: 0.35,
                },
            };
            let window = HandoffWindow {
                edges: [0, 1].map(|i| r.authoring_edge_chain(lanes[i]).unwrap()[0]),
                owner: BoundaryOwner::Section(section),
                left_index: 0,
                stations_mm: [20_000, 50_000],
            };
            let tolerance = 1_000_000;
            let query = |target, limit| n.handoff_travel(&q, window, target, tolerance, limit);
            let mut all = Vec::new();
            for advance in [0, 15 * NM, 30 * NM] {
                let s = if start < end {
                    start + advance
                } else {
                    start - advance
                };
                let target = HandoffTravelTarget::RearStationNm(s);
                let report = query(target, TRACK_WORK_LIMIT);
                let result = report
                    .bounds
                    .unwrap_or_else(|_| panic!("{report:?}, s={s}"));
                if advance == 0 {
                    assert_eq!(
                        result,
                        HandoffTravelBounds {
                            distance_nm: [0; 2],
                            rear_stations_nm: [start; 2]
                        }
                    );
                } else {
                    let [lo, hi] = oracle(advance as f64 / NM as f64);
                    assert!(
                        result.distance_nm[0] as f64 <= lo * NM as f64
                            && result.distance_nm[1] as f64 >= hi * NM as f64,
                        "{result:?}, {lo}..{hi}"
                    );
                    assert!(result.distance_nm[0] > advance);
                }
                assert_eq!(result.rear_stations_nm, [s; 2]);
                assert!(result.distance_nm[1] - result.distance_nm[0] <= tolerance);
                assert_eq!(query(target, report.work), report);
                assert_eq!(
                    query(target, report.work - 1).bounds,
                    Err(Failure::WorkLimit)
                );
                all.push(report);
            }
            let zero = query(HandoffTravelTarget::DistanceNm(0), TRACK_WORK_LIMIT);
            assert_eq!(zero, all[0]);
            let ordered = [0, 15 * NM, 30 * NM];
            let marks = |offsets: &[u64], work| {
                n.handoff_travel_marks(&q, window, offsets, tolerance, work)
            };
            let batch = marks(&ordered, TRACK_WORK_LIMIT);
            let distances = batch
                .distances_nm
                .as_ref()
                .unwrap_or_else(|_| panic!("{batch:?}"));
            assert_eq!(distances[0], [0; 2]);
            for (i, &[lo, hi]) in distances.iter().enumerate().skip(1) {
                let analytic = oracle(ordered[i] as f64 / NM as f64);
                assert!(
                    (lo as f64) <= analytic[0] * NM as f64
                        && (hi as f64) >= analytic[1] * NM as f64
                );
                let single = all[i].bounds.unwrap().distance_nm;
                assert!(lo <= single[1] && hi >= single[0] && hi - lo <= tolerance);
            }
            assert!(batch.work < all.iter().map(|r| r.work).sum::<u32>());
            assert_eq!(marks(&ordered, batch.work), batch);
            assert_eq!(
                marks(&ordered, batch.work - 1).distances_nm,
                Err(Failure::WorkLimit)
            );
            for bad in [&[][..], &[0, NM], &[30 * NM, 30 * NM], &[31 * NM, 30 * NM]] {
                assert_eq!(
                    marks(bad, TRACK_WORK_LIMIT).distances_nm,
                    Err(Failure::InvalidInput)
                );
            }
            // 标记核验和几何积分共享预算；在预算足够读到错误标记时就报告输入错误。
            let reversed = [2 * NM, NM, 30 * NM];
            let invalid = marks(&reversed, 2);
            assert_eq!(invalid.distances_nm, Err(Failure::InvalidInput));
            assert_eq!(invalid.work, 2);
            let short = marks(&reversed, 1);
            assert_eq!(short.distances_nm, Err(Failure::WorkLimit));
            assert_eq!(short.work, 1);
            for d in [10 * NM, 15_144_572_165] {
                let target = HandoffTravelTarget::DistanceNm(d);
                let inverse = query(target, TRACK_WORK_LIMIT);
                let result = inverse.bounds.unwrap_or_else(|_| panic!("{inverse:?}"));
                assert_eq!(result.distance_nm, [d; 2]);
                let [a, b] = result
                    .rear_stations_nm
                    .map(|s| start.abs_diff(s) as f64 / NM as f64);
                assert!(
                    oracle(a.min(b))[1] * NM as f64 <= d as f64
                        && oracle(a.max(b))[0] * NM as f64 >= d as f64,
                    "{result:?}"
                );
                assert!(result.rear_stations_nm[1] - result.rear_stations_nm[0] <= tolerance);
                all.push(inverse);
                assert_eq!(query(target, inverse.work), inverse);
                assert_eq!(
                    query(target, inverse.work - 1).bounds,
                    Err(Failure::WorkLimit)
                );
            }
            assert_eq!(
                query(HandoffTravelTarget::RearStationNm(9 * NM), TRACK_WORK_LIMIT).bounds,
                Err(Failure::InvalidInput)
            );
            assert_eq!(
                query(HandoffTravelTarget::DistanceNm(40 * NM), TRACK_WORK_LIMIT).bounds,
                Err(Failure::InvalidInput)
            );
            let end_station = HandoffTravelTarget::RearStationNm(end);
            let unresolved = n.handoff_travel(&q, window, end_station, 1, u32::MAX);
            assert!(matches!(
                unresolved.bounds,
                Err(Failure::TravelResolution | Failure::WorkLimit)
            ));
            assert_eq!(
                n.handoff_travel(&q, window, end_station, 1, TRACK_WORK_LIMIT),
                unresolved
            );
            assert_eq!(
                n.handoff_travel(&q, window, end_station, 0, TRACK_WORK_LIMIT)
                    .bounds,
                Err(Failure::InvalidInput)
            );
            if let Some(prior) = previous.replace(all.clone()) {
                assert_eq!(prior, all);
            }
        }
    }
}
