#![cfg(feature = "adapter")]

use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_spatial::{FramePlacementToken, SpatialSession};
use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{Artifacts, Harness, Presentation, ResolvedPlan, UrbanCase, Window};

#[test]
fn selected_and_full_extraction_apply_the_same_dynamic_host_selection() {
    use bevy_transform::components::Transform;
    use laneflow_urban_harness::{PresentationMode, SelectionWindow};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    generate(&config, Scale::Fixture, &source, None).unwrap();
    let artifacts = Artifacts::load_spatial(&source).unwrap();
    let plan = ResolvedPlan::for_case(&artifacts, UrbanCase::MixedPeak, Window::probe(32).unwrap())
        .unwrap();
    let install = || {
        Harness::install(
            &artifacts,
            &plan,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap()
        .into_adapter(
            SpatialSession::bind(artifacts.revision().clone())
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    };
    for stride in [0, 137] {
        for percent in [0, 1, 10, 100] {
            let selection = SelectionWindow {
                percent,
                offset: 53,
                stride,
                reverse: true,
            };
            let mut full = install();
            let mut selected = install();
            let mut full_presentation =
                Presentation::new(PresentationMode::FullValidationSelected { selection }).unwrap();
            let mut selected_presentation =
                Presentation::new(PresentationMode::SelectedPresentation { selection }).unwrap();
            for tick in 0..32 {
                assert_eq!(full.advance().unwrap(), selected.advance().unwrap());
                let token = FramePlacementToken::new(tick + 7);
                let full_sample = full_presentation
                    .sample_with_placement(&mut full, token)
                    .unwrap();
                let selected_sample = selected_presentation
                    .sample_with_placement(&mut selected, token)
                    .unwrap();
                assert_eq!(full_sample.presentable, full_sample.extracted);
                assert_eq!(full_sample.applied, selected_sample.applied);
                assert_eq!(full_sample.applied_digest, selected_sample.applied_digest);
                assert_eq!(selected_sample.applied, selected_sample.extracted);
                assert_eq!(
                    selected_sample.requested,
                    selected_sample.individual * usize::from(percent) / 100
                );
                assert_eq!(selected_sample.n_presented, selected_sample.extracted);
                assert_eq!(full_sample.n_presented, full_sample.extracted);
                let handles = full.world().live_vehicles().to_vec();
                for handle in handles {
                    let transform = |h: &mut Harness<'_>| {
                        let world = h.adapter_world().unwrap();
                        world
                            .resource::<LaneFlowSession>()
                            .vehicle_entity(handle)
                            .and_then(|entity| world.get::<Transform>(entity))
                            .copied()
                    };
                    assert_eq!(transform(&mut full), transform(&mut selected));
                }
            }
            assert_eq!(full.checkpoint().unwrap(), selected.checkpoint().unwrap());
        }
    }
    assert!(
        Presentation::new(PresentationMode::SelectedPresentation {
            selection: SelectionWindow {
                percent: 101,
                offset: 0,
                stride: 0,
                reverse: false
            },
        })
        .is_err()
    );
}

#[test]
fn bevy_and_headless_share_demand_and_committed_results() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    generate(&config, Scale::Fixture, &source, None).unwrap();
    let artifacts = Artifacts::load_spatial(&source).unwrap();
    for case in UrbanCase::ALL {
        let plan = ResolvedPlan::for_case(&artifacts, case, Window::probe(128).unwrap()).unwrap();
        let mut headless = Harness::install(
            &artifacts,
            &plan,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap();
        let spatial = SpatialSession::bind(artifacts.revision().clone())
            .unwrap()
            .unwrap();
        let mut adapter = Harness::install(
            &artifacts,
            &plan,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap()
        .into_adapter(spatial)
        .unwrap();
        let mut poses = LaneFlowCommittedPoseBatch::new();
        let mut presentation = Presentation::default();
        for _ in 0..128 {
            assert_eq!(
                headless.advance().unwrap(),
                adapter.advance().unwrap(),
                "{}",
                case.as_str()
            );
            let expected = adapter
                .world()
                .committed_pose_sources()
                .collect::<Vec<_>>()
                .as_slice()
                .len();
            adapter
                .adapter_world()
                .unwrap()
                .resource_mut::<LaneFlowSession>()
                .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
                .unwrap();
            assert_eq!(poses.vehicles().len(), expected);
            assert_eq!(poses.batch().records().len(), expected);
            assert!(
                expected < 2_000,
                "virtual parked vehicles must have no pose"
            );
            let sample = presentation.sample(&mut adapter).unwrap();
            assert_eq!(sample.extracted, expected);
            assert_eq!(sample.applied, expected);
            assert_eq!(sample.n_presented, expected);
        }
        assert_eq!(
            headless.checkpoint().unwrap(),
            adapter.checkpoint().unwrap()
        );
    }
    let second = Artifacts::load_spatial(&source).unwrap();
    let wrong = SpatialSession::bind(second.revision().clone())
        .unwrap()
        .unwrap();
    let plan = ResolvedPlan::mixed(&artifacts, Window::probe(16).unwrap()).unwrap();
    assert!(
        Harness::install(
            &artifacts,
            &plan,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN)
        )
        .unwrap()
        .into_adapter(wrong)
        .is_err()
    );
}
