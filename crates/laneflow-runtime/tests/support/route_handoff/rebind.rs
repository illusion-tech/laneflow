use super::*;
use laneflow_compiler::{
    CompilationUnitBuilder, Compiler, PortableDiffBase, PortableEmissionProvenance,
    derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{
    FormatLimits, ImmutableObjectSource, PostEmissionCheckedBundle, check_post_emission_bundle,
};
use laneflow_runtime::{TrafficWorld, deterministic_state_digest};
use laneflow_static_contract::{EntityKind, LaneEdgeId, LaneEdgeOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
    vehicle::{Failure, HandoffSourceEvidence, HandoffSourceModule},
};

const ROAD_NS: &str = "city/runtime-conflict";
const OTHER_NS: &str = "city/other";
const ROAD_DOC: &str = "runtime-conflict.lfre";
const OTHER_DOC: &str = "other.lfre";

fn other_module(extra_road: bool) -> RoadEditingSourceModule {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let header = RoadEditingModuleHeader::try_new(
        OTHER_NS,
        OTHER_DOC,
        vec![],
        RoadEditingProvenance::direct("outside handoff").unwrap(),
    )
    .unwrap();
    let mut b = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &limits,
    )
    .unwrap();
    b.add_declaration(RoadEditingDeclaration::CanonicalFrame(
        CanonicalFrameInput::try_new("frame-main").unwrap(),
    ))
    .unwrap();
    add_road_editing_approach(
        &mut b,
        "elsewhere",
        road_editing_line((200., 0.), (200., 40.)),
        vec![],
    );
    if extra_road {
        add_road_editing_approach(
            &mut b,
            "new-road",
            road_editing_line((210., 0.), (210., 40.)),
            vec![],
        );
    }
    b.finish().unwrap()
}

fn compile(
    sources: &[HandoffSourceModule<'_>],
) -> (
    TrafficWorld,
    PostEmissionCheckedBundle<ImmutableObjectSource, ImmutableObjectSource, ImmutableObjectSource>,
) {
    let mut unit =
        CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
    for s in sources {
        unit.add_road_editing_module(
            RoadEditingModuleInput::try_new(s.document_key, s.source_bytes, None).unwrap(),
        )
        .unwrap();
    }
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("rebind-route").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let owned = |bytes: &[u8]| ImmutableObjectSource::from_boxed_bytes(bytes.into());
    let checked = check_post_emission_bundle(
        owned(candidate.canonical_artifact().bytes()),
        owned(candidate.source_map().bytes()),
        owned(candidate.semantic_diff().bytes()),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    let network = build_shared_network_revision(
        checked.clone().canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1024 * 1024, 16 * 1024 * 1024),
        ),
    )
    .unwrap();
    (
        install_fixture(network, WorldConfig::new(2, 4, 64, 32, 100)).unwrap(),
        checked,
    )
}

fn edge(world: &TrafficWorld, key: &str) -> LaneEdgeOrdinal {
    let stable = derive_canonical_stable_id_v1(EntityKind::LaneEdge, ROAD_NS, key).unwrap();
    world
        .revision()
        .identity()
        .ordinal(LaneEdgeId::from_untyped(stable))
        .unwrap()
}

#[test]
fn rebinds_the_exact_repeated_occurrence_across_an_unrelated_road_edit() {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let write = |module| RoadEditingSourceWriter::new(&limits).write(module).unwrap();
    let road = write(module(0.));
    // 仅平移参考线，边稳定标识、方向、局部长度及原交接窗口仍匹配。
    let shifted_road = write(module(1.));
    let other = write(other_module(false));
    let expanded = write(other_module(true));
    let base_modules = [
        HandoffSourceModule {
            namespace: OTHER_NS,
            document_key: OTHER_DOC,
            source_bytes: other.as_bytes(),
        },
        HandoffSourceModule {
            namespace: ROAD_NS,
            document_key: ROAD_DOC,
            source_bytes: road.as_bytes(),
        },
    ];
    let new_modules = [
        HandoffSourceModule {
            source_bytes: expanded.as_bytes(),
            ..base_modules[0]
        },
        base_modules[1],
    ];
    let shifted_modules = [
        new_modules[0],
        HandoffSourceModule {
            source_bytes: shifted_road.as_bytes(),
            ..new_modules[1]
        },
    ];
    let (mut base, base_artifact) = compile(&base_modules);
    let (mut target, target_artifact) = compile(&new_modules);
    let (mut shifted, shifted_artifact) = compile(&shifted_modules);
    assert_ne!(
        base.revision().network_revision(),
        target.revision().network_revision()
    );
    assert_ne!(
        target.revision().network_revision(),
        shifted.revision().network_revision()
    );
    for world in [&mut base, &mut target, &mut shifted] {
        let regular = RouteRegisterInput::new(vec![edge(world, "s0"), edge(world, "s1")]);
        world.register_route(regular).unwrap();
    }
    let state = |world: &TrafficWorld| {
        (
            world.command_cursor(),
            world.event_cursor(),
            world.live_routes().collect::<Vec<_>>(),
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        )
    };
    let before = [&base, &target, &shifted].map(state);
    let old =
        HandoffSourceEvidence::check(&base.revision(), &base_artifact, &base_modules[1..]).unwrap();
    let new = HandoffSourceEvidence::check(&target.revision(), &target_artifact, &new_modules[1..])
        .unwrap();
    let changed = HandoffSourceEvidence::check(
        &shifted.revision(),
        &shifted_artifact,
        &shifted_modules[1..],
    )
    .unwrap();
    let route = RouteRegisterInput::new(
        ["t1", "t2", "return", "s0", "s1", "s2", "t1", "t2"]
            .map(|k| edge(&base, k))
            .to_vec(),
    );
    let r = base.traffic().relations();
    let source_lane = r.lane_edge_authoring_lane(edge(&base, "s2")).unwrap();
    let occurrence = RouteHandoffOccurrence {
        revision: base.revision().network_revision(),
        source_index: 5,
        owner: BoundaryOwner::Section(r.authoring_section(source_lane).unwrap()),
        left_index: 0,
        stations_nm: [24_000_000_000, 26_000_000_000],
    };
    let binding = base.bind_route_handoff(&route, occurrence).unwrap();
    let rebound = target.rebind_route_handoff(&binding, &old, &new).unwrap();
    assert_eq!(
        rebound.boundary_at(5).unwrap().revision(),
        target.revision().network_revision()
    );
    assert_eq!(
        rebound.boundary_at(5).unwrap().window().stations_nm,
        occurrence.stations_nm
    );
    let expected_edges = ["s2", "t1"].map(|k| edge(&target, k));
    assert_ne!(
        binding.boundary_at(5).unwrap().window().edges,
        expected_edges
    );
    assert_eq!(
        rebound.boundary_at(5).unwrap().window().edges,
        expected_edges
    );
    let trial = RouteHandoffTrial {
        profile: base
            .revision()
            .identity()
            .stable_id(conflict_profile(&base.revision(), "car"))
            .unwrap(),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    };
    let preview = target.preview_route_handoff(&rebound, &trial).unwrap();
    assert_eq!(preview.source_index, 5);
    let HandoffOutcome::Located { addresses, .. } = preview.report.outcome else {
        panic!("{preview:?}")
    };
    assert_eq!(addresses.map(|a| a.edge), expected_edges);
    let repeated = RouteRegisterInput::new(
        ["s2", "t1", "t2", "return", "s0", "s1", "s2", "t1"]
            .map(|k| edge(&base, k))
            .to_vec(),
    );
    let both = base
        .bind_route_handoffs(
            &repeated,
            &[
                RouteHandoffOccurrence {
                    source_index: 0,
                    stations_nm: [24_000_000_000, 25_000_000_000],
                    ..occurrence
                },
                RouteHandoffOccurrence {
                    source_index: 6,
                    ..occurrence
                },
            ],
        )
        .unwrap();
    let rebound_both = target.rebind_route_handoff(&both, &old, &new).unwrap();
    for &(index, boundary) in both.windows() {
        let restored = rebound_both.boundary_at(index).unwrap();
        assert_eq!(restored.window().edges, expected_edges);
        assert_eq!(restored.window().stations_nm, boundary.window().stations_nm);
    }
    let first = target
        .preview_route_handoff_at(&rebound_both, 0, &trial)
        .unwrap();
    assert!(matches!(
        first.report.outcome,
        HandoffOutcome::OutsideWindow { .. }
    ));
    let second = target
        .preview_route_handoff_at(&rebound_both, 6, &trial)
        .unwrap();
    assert!(matches!(
        second.report.outcome,
        HandoffOutcome::Located { .. }
    ));
    assert_eq!(
        target.preview_route_handoff(&binding, &trial).unwrap_err(),
        RouteHandoffTrialError::RevisionMismatch
    );
    assert_eq!(
        shifted
            .rebind_route_handoff(&binding, &old, &changed)
            .unwrap_err(),
        RouteHandoffTrialError::Boundary(Failure::Unavailable)
    );
    for (base_evidence, target_evidence) in [(&old, &old), (&new, &new)] {
        assert_eq!(
            target
                .rebind_route_handoff(&binding, base_evidence, target_evidence)
                .unwrap_err(),
            RouteHandoffTrialError::Boundary(Failure::Binding)
        );
    }
    assert!(matches!(
        HandoffSourceEvidence::check(&target.revision(), &target_artifact, &shifted_modules[1..]),
        Err(Failure::Binding)
    ));
    assert!(matches!(
        HandoffSourceEvidence::check(&target.revision(), &base_artifact, &new_modules[1..]),
        Err(Failure::Binding)
    ));
    assert_eq!(before, [&base, &target, &shifted].map(state));
}
