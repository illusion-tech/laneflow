//! 无 Gate 的人口测试路网：保留 catalog 的入口、槽位与权重，每条路线只走入口边。
//! 两套显式策略与 NotRequired 均可合法安装，隔离策略选择对生命周期绑定的影响。
use std::{collections::BTreeSet, sync::Arc};

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{PolicyPin, WorldPolicySelection};
use laneflow_scenario::signalized_corridor::{
    AUTHORING_NAMESPACE, CatalogPolicySelection, CorridorCatalog, PASSENGER_CAR_PROFILE_KEY,
    SHUTTLE_BUS_PROFILE_KEY,
};
use laneflow_static_contract::RightOfWayPolicySetOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

pub fn fixture(
    mut catalog: CorridorCatalog,
) -> (
    Arc<SharedNetworkRevision>,
    CorridorCatalog,
    [WorldPolicySelection; 3],
) {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: AUTHORING_NAMESPACE,
            source_document_key: "population-policy.document",
            generator_build_id: "population-policy-test",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: None,
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap();
    for key in [PASSENGER_CAR_PROFILE_KEY, SHUTTLE_BUS_PROFILE_KEY] {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: key,
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .unwrap();
    }
    for key in catalog
        .spawn_slots
        .iter()
        .map(|slot| slot.edge_id.as_str())
        .collect::<BTreeSet<_>>()
    {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 500.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[],
            })
            .unwrap();
    }
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            let entry = catalog
                .spawn_slots
                .iter()
                .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
                .unwrap();
            for choice in &lane.route_choices {
                let route = catalog
                    .routes
                    .iter_mut()
                    .find(|route| route.route_id == choice.route_id)
                    .unwrap();
                route.edge_ids = vec![entry.edge_id.clone()];
            }
        }
    }
    let span = module.policy_source_span();
    for key in ["policy-a", "policy-b"] {
        module
            .add_right_of_way_policy_set(RightOfWayPolicySetInput {
                policy_set_key: key,
                regulation: RegulationIdentity {
                    jurisdiction: "engineering",
                    version: "population-fixture-v1",
                    source: Some("repository:population-policy-fixture"),
                },
                evidence: &[],
                gap_profiles: &[],
                stream_rules: &[],
                gate_rules: &[],
                source: PolicyInputSource {
                    primary: &span,
                    contributing: &[],
                },
            })
            .unwrap();
    }
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("population-policy-v1").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let pins = [0, 1].map(|ordinal| {
        WorldPolicySelection::Pinned(PolicyPin {
            policy: revision
                .identity()
                .stable_id(RightOfWayPolicySetOrdinal::from_raw(ordinal))
                .unwrap(),
        })
    });
    (
        revision,
        catalog,
        [pins[0], pins[1], WorldPolicySelection::NotRequired],
    )
}

pub fn select(catalog: &mut CorridorCatalog, selection: WorldPolicySelection) {
    catalog.policy_selection = match selection {
        WorldPolicySelection::NotRequired => CatalogPolicySelection::NotRequired {},
        WorldPolicySelection::Pinned(pin) => CatalogPolicySelection::Pinned {
            policy: pin.policy.to_string(),
        },
    };
}
