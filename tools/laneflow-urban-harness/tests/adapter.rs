#![cfg(feature = "adapter")]

use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_spatial::{FramePlacementToken, SpatialSession};
use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{Artifacts, Harness, Presentation, ResolvedPlan, UrbanCase, Window};

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
