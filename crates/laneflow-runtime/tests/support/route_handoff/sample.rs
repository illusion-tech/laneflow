use super::*;
use laneflow_runtime::{RouteHandoffPreview, RouteHandoffSampleError, deterministic_state_digest};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::vehicle::{Failure, HandoffLaneSide};

const TOLERANCE_NM: u64 = 5_000_000;
const LIMITS: GeometryWorkLimits = GeometryWorkLimits::STANDARD;

fn check_occurrences(
    preview: &RouteHandoffPreview<'_>,
    source_index: u32,
    physical_edges: [LaneEdgeOrdinal; 4],
) {
    use HandoffLaneSide::{Source, Target};
    let cases = [
        (0, 0, source_index - 1, Source, [true, false]),
        (0, 10_000_000, source_index, Source, [true, true]),
        (12_999_999, 2, source_index + 1, Target, [true, true]),
        (
            13_000_001,
            9_999_999,
            source_index + 2,
            Target,
            [false, true],
        ),
    ];
    for ((before, step, occurrence, side, occupied), edge) in cases.into_iter().zip(physical_edges)
    {
        let report = preview
            .sample_after_travel(before, step, TOLERANCE_NM, LIMITS)
            .unwrap();
        let sample = report
            .sample
            .as_ref()
            .unwrap_or_else(|_| panic!("{report:?}"));
        assert_eq!(sample.rear_travel_um, before + step);
        assert_eq!(sample.front_occurrence, occurrence);
        assert_eq!(sample.footprint.front_side, side);
        assert_eq!(
            sample.footprint.sides.each_ref().map(Option::is_some),
            occupied
        );
        let mut addresses = sample.footprint.front.addresses();
        let front = addresses.next().unwrap();
        assert_eq!(front.edge, edge);
        assert!(addresses.next().is_none());
        if before == 0 && step == 0 {
            assert_eq!(sample.rear_stations_nm, [10_000_000_000; 2]);
            // s1 从 10 m 开始，静止的前杠位于后轴前方 3.40 m。
            assert!((front.progress_mm[0]..=front.progress_mm[1]).contains(&3400));
        }
        assert_eq!(
            preview
                .sample_after_travel(sample.rear_travel_um, 0, TOLERANCE_NM, LIMITS)
                .as_ref(),
            Some(&report)
        );
    }
}

fn check_query_limits(preview: &RouteHandoffPreview<'_>) {
    let sample = |limits| {
        preview
            .sample_after_travel(12_999_999, 2, TOLERANCE_NM, limits)
            .unwrap()
    };
    let full = sample(LIMITS);
    assert!(full.sample.is_ok());
    let exact = GeometryWorkLimits {
        tracks: full.travel_work,
        sweep: full.footprint_work,
    };
    assert_eq!(sample(exact), full);
    let travel_short = sample(GeometryWorkLimits {
        tracks: exact.tracks - 1,
        ..exact
    });
    assert_eq!(
        travel_short.sample,
        Err(RouteHandoffSampleError::Travel(Failure::WorkLimit))
    );
    assert_eq!(
        (travel_short.travel_work, travel_short.footprint_work),
        (exact.tracks - 1, 0)
    );
    let body_short = sample(GeometryWorkLimits {
        sweep: exact.sweep - 1,
        ..exact
    });
    assert_eq!(
        body_short.sample,
        Err(RouteHandoffSampleError::Footprint(Failure::WorkLimit))
    );
    assert_eq!(
        (body_short.travel_work, body_short.footprint_work),
        (exact.tracks, exact.sweep - 1)
    );

    // 加法溢出、微米到纳米的首次溢出均在几何查询前拒绝。
    for (before, step) in [(u64::MAX, 1), (u64::MAX / 1000, 1)] {
        let report = preview
            .sample_after_travel(before, step, TOLERANCE_NM, LIMITS)
            .unwrap();
        assert_eq!(
            report.sample,
            Err(RouteHandoffSampleError::DistanceOverflow)
        );
        assert_eq!((report.travel_work, report.footprint_work), (0, 0));
    }
    let outside = preview
        .sample_after_travel(30_000_000, 1_000_000, TOLERANCE_NM, LIMITS)
        .unwrap();
    assert_eq!(
        outside.sample,
        Err(RouteHandoffSampleError::Travel(Failure::InvalidInput))
    );
    assert_eq!(outside.footprint_work, 0);

    // 在源车道 22 m 切点，独立计算的前杠纵站为 s + 3.4 / sqrt(1 + y'²)。
    // 五次横移的后轴纵站为 18.63619435 m，积分至此的行程为 8.66079991 m。
    let at_cut = preview
        .sample_after_travel(8_660_800, 0, TOLERANCE_NM, LIMITS)
        .unwrap();
    assert_eq!(at_cut.sample, Err(RouteHandoffSampleError::AmbiguousFront));
}

#[test]
fn sampled_travel_keeps_body_evidence_and_counts_only_the_selected_route_visit() {
    let rev = compile_road_editing_revision(module(0.));
    let world = install_fixture(
        Arc::clone(&rev),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 2,
            route_capacity: 4,
            route_edge_occurrence_capacity: 64,
            route_conflict_occurrence_capacity: 32,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let rel = rev.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&section| rel.section_lanes(section).unwrap().len() == 2)
        .unwrap();
    let lanes = rel.section_lanes(section).unwrap();
    let src = rel.authoring_edge_chain(lanes[0]).unwrap();
    let dst = rel.authoring_edge_chain(lanes[1]).unwrap();
    let turn = rev.traffic().successors(dst[2]).unwrap()[0];
    let cursor = world.command_cursor();
    let digest = || deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    let before = digest();
    let occurrence = RouteHandoffOccurrence {
        revision: rev.network_revision(),
        source_index: 2,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: [24_000_000_000, 26_000_000_000],
    };
    let trial = RouteHandoffTrial {
        profile: rev
            .identity()
            .stable_id(conflict_profile(&rev, "car"))
            .unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 1.,
        limits: LIMITS,
    };
    let route = RouteRegisterInput::new(vec![src[0], src[1], src[2], dst[1], dst[2]]);
    let binding = world.bind_route_handoff(&route, occurrence).unwrap();
    let preview = world.preview_route_handoff(&binding, &trial).unwrap();
    let edges = [src[1], src[2], dst[1], dst[2]];
    check_occurrences(&preview, 2, edges);
    check_query_limits(&preview);

    let second_visit = RouteRegisterInput::new(vec![
        dst[1], dst[2], turn, src[0], src[1], src[2], dst[1], dst[2],
    ]);
    let second_occurrence = RouteHandoffOccurrence {
        source_index: 5,
        ..occurrence
    };
    let second_binding = world
        .bind_route_handoff(&second_visit, second_occurrence)
        .unwrap();
    let second = world
        .preview_route_handoff(&second_binding, &trial)
        .unwrap();
    check_occurrences(&second, 5, edges);

    // 同一目标边 t2 在本次出现之后缺失，但此前访问过它；不能回退到上一圈。
    let truncated = RouteRegisterInput::new(second_visit.edges()[..7].to_vec());
    let truncated_binding = world
        .bind_route_handoff(&truncated, second_occurrence)
        .unwrap();
    let missing_target = world
        .preview_route_handoff(&truncated_binding, &trial)
        .unwrap()
        .sample_after_travel(23_000_000, 0, TOLERANCE_NM, LIMITS)
        .unwrap();
    assert_eq!(
        missing_target.sample,
        Err(RouteHandoffSampleError::FrontOutsideRoute { edge: dst[2] })
    );
    let short = RouteRegisterInput::new(vec![src[2], dst[1]]);
    let short_binding = world
        .bind_route_handoff(
            &short,
            RouteHandoffOccurrence {
                source_index: 0,
                ..occurrence
            },
        )
        .unwrap();
    let missing_source = world
        .preview_route_handoff(&short_binding, &trial)
        .unwrap()
        .sample_after_travel(0, 0, TOLERANCE_NM, LIMITS)
        .unwrap();
    assert_eq!(
        missing_source.sample,
        Err(RouteHandoffSampleError::FrontOutsideRoute { edge: src[1] })
    );

    let narrow = world
        .bind_route_handoff(
            &route,
            RouteHandoffOccurrence {
                stations_nm: [24_000_000_000, 25_000_000_000],
                ..occurrence
            },
        )
        .unwrap();
    let rejected = world.preview_route_handoff(&narrow, &trial).unwrap();
    assert!(matches!(
        rejected.report.outcome,
        HandoffOutcome::OutsideWindow { .. }
    ));
    assert!(
        rejected
            .sample_after_travel(0, 0, TOLERANCE_NM, LIMITS)
            .is_none()
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_routes().count(), 0);
    assert_eq!(digest(), before);
}
