use super::*;
use laneflow_static_network::{StationAddress, vehicle::PoseBounds};

#[test]
fn sparse_handoff_uses_projected_ground_and_banked_axes_and_rejects_bad_batches_atomically() {
    let mut session = bound_session();
    let inputs = lane_inputs(2);
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(1), &inputs, &mut output)
        .unwrap();
    let ordinary = output.records()[0];
    let frame = output.canonical_frame().unwrap();
    let revision = session.network_revision();
    let exact = |v: [f64; 3]| v.map(|x| [x, x]);
    let tilted = PoseBounds {
        rear_m: [[0.; 2]; 3],
        forward: exact([1., 0., 0.]),
        left: exact([0., 0.6, -0.8]),
        up: exact([0., 0.8, 0.6]),
    };
    let handoff = || HandoffPoseInput {
        input_index: 1,
        revision,
        frame,
        pose: &tilted,
        front: Some(StationAddress {
            edge: EDGE_A0,
            progress_mm: [37, 37],
        }),
        front_ground_mm: Some([[1000, 1000], [201, 202], [-401, -400]]),
    };
    session
        .extract_pose_batch_with_handoffs(token(2), &inputs, [handoff()].into_iter(), &mut output)
        .unwrap();
    assert_eq!(output.records()[0], ordinary);
    let pose = output.records()[1].pose();
    assert_eq!(
        [
            pose.position().x(),
            pose.position().y(),
            pose.position().z()
        ],
        [1., 0.2015, -0.4005]
    );
    assert_eq!(
        [pose.up().x(), pose.up().y(), pose.up().z()],
        [0., 0.8, 0.6]
    );
    let saved = output.clone();
    let flipped = PoseBounds {
        up: exact([0., -0.8, -0.6]),
        ..tilted
    };
    let skewed = PoseBounds {
        left: exact([0.6, 0., -0.8]),
        ..tilted
    };
    let foreign = NetworkRevisionId::from_digest(laneflow_static_contract::Sha256Digest::ZERO);
    for invalid in [
        vec![HandoffPoseInput {
            revision: foreign,
            ..handoff()
        }],
        vec![HandoffPoseInput {
            frame: CanonicalFrameOrdinal::from_raw(frame.raw() + 1),
            ..handoff()
        }],
        vec![HandoffPoseInput {
            pose: &flipped,
            ..handoff()
        }],
        vec![HandoffPoseInput {
            pose: &skewed,
            ..handoff()
        }],
        vec![HandoffPoseInput {
            front: Some(StationAddress {
                edge: EDGE_A0,
                progress_mm: [0, 0],
            }),
            ..handoff()
        }],
        vec![HandoffPoseInput {
            front: None,
            ..handoff()
        }],
        vec![HandoffPoseInput {
            front_ground_mm: None,
            ..handoff()
        }],
        vec![handoff(), handoff()],
        vec![HandoffPoseInput {
            input_index: inputs.len(),
            ..handoff()
        }],
    ] {
        assert!(
            session
                .extract_pose_batch_with_handoffs(
                    token(3),
                    &inputs,
                    invalid.into_iter(),
                    &mut output
                )
                .is_err()
        );
        assert_eq!(output, saved);
        assert!(session.scratch.is_empty());
    }
}
