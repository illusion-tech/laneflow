//! Spatial 位姿批次提交的公共语义合同测试。
//!
//! 覆盖成功批次、失败原子性、空批次、重试、交替 output 与 Lane/Parking 混合来源。
//! 这些测试只依赖公开 API，必须在成功提交机制（复制或所有权交换）的任何实现上
//! 等价通过；缓冲归属与容量轮换由 `session.rs` 模块内测试另行验证。

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_spatial::{
    CanonicalPoseBatch, CanonicalPoseRecord, FramePlacementToken, PoseInput, PoseRecordId,
    SpatialError, SpatialSession,
};
use laneflow_static_contract::{CanonicalFrameOrdinal, LaneEdgeOrdinal, ParkingSpaceOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);

/// frame 0 内的边（长 12000 mm）。
const EDGE_A0: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(0);
/// frame 0 内的边（长 10000 mm）。
const EDGE_A1: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(1);
/// frame 1 内的边（长 12000 mm）。
const EDGE_B0: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(3);
/// 共享根内不存在的边。
const EDGE_UNKNOWN: LaneEdgeOrdinal = LaneEdgeOrdinal::from_raw(999);
/// 共享根内唯一的显式停车位。
const PARKING0: ParkingSpaceOrdinal = ParkingSpaceOrdinal::from_raw(0);

fn revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn frame_of(revision: &SharedNetworkRevision, edge: LaneEdgeOrdinal) -> CanonicalFrameOrdinal {
    revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .and_then(|network| network.lane_geometry(edge))
        .expect("lane geometry")
        .canonical_frame()
}

fn lane_length_mm(revision: &SharedNetworkRevision, edge: LaneEdgeOrdinal) -> u32 {
    revision.traffic().lane_lengths_millimetres()[edge.index()]
}

/// 断言两条记录的位姿各浮点分量位模式一致（本任务不改变浮点计算）。
fn assert_records_bit_identical(left: &[CanonicalPoseRecord], right: &[CanonicalPoseRecord]) {
    assert_eq!(left.len(), right.len(), "record count");
    for (index, (left, right)) in left.iter().zip(right).enumerate() {
        assert_eq!(left.record(), right.record(), "record id at {index}");
        let (left, right) = (left.pose(), right.pose());
        let (lp, rp) = (left.position(), right.position());
        assert_eq!(
            (
                lp.x().to_bits(),
                lp.y().to_bits(),
                lp.z().to_bits(),
                left.tangent().x().to_bits(),
                left.tangent().y().to_bits(),
                left.tangent().z().to_bits(),
                left.up().x().to_bits(),
                left.up().y().to_bits(),
                left.up().z().to_bits(),
            ),
            (
                rp.x().to_bits(),
                rp.y().to_bits(),
                rp.z().to_bits(),
                right.tangent().x().to_bits(),
                right.tangent().y().to_bits(),
                right.tangent().z().to_bits(),
                right.up().x().to_bits(),
                right.up().y().to_bits(),
                right.up().z().to_bits(),
            ),
            "pose bits at {index}"
        );
    }
}

fn input_lane(record: u32, edge: LaneEdgeOrdinal, progress_mm: u32) -> PoseInput {
    PoseInput::lane(PoseRecordId::new(record), edge, progress_mm)
}

/// S01：正常非空批次的修订、frame、token、记录数量、身份、顺序与位姿基线。
#[test]
fn s01_nonempty_batch_matches_shared_root_baseline() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(41),
            &[
                input_lane(7, EDGE_A0, 0),
                input_lane(900, EDGE_A1, 5_000),
                input_lane(3, EDGE_A0, lane_length_mm(&revision, EDGE_A0)),
            ],
            &mut output,
        )
        .expect("extract");

    assert_eq!(output.network_revision(), Some(session.network_revision()));
    assert_eq!(output.canonical_frame(), Some(frame_of(&revision, EDGE_A0)));
    assert_eq!(output.placement_token(), FramePlacementToken::new(41));
    assert_eq!(
        output
            .records()
            .iter()
            .map(|record| record.record())
            .collect::<Vec<_>>(),
        vec![
            PoseRecordId::new(7),
            PoseRecordId::new(900),
            PoseRecordId::new(3)
        ]
    );
    // 位姿基线来自共享根公开几何：进度 0 落在首点，满进度落在末点。
    let geometry = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .and_then(|network| network.lane_geometry(EDGE_A0))
        .expect("lane geometry");
    let first = geometry.points()[0];
    let last = *geometry.points().last().expect("end point");
    let start = output.records()[0].pose().position();
    let end = output.records()[2].pose().position();
    assert_eq!(
        (start.x(), start.y(), start.z()),
        (first.x, first.y, first.z)
    );
    assert_eq!((end.x(), end.y(), end.z()), (last.x, last.y, last.z));

    // 独立会话对同一输入产生位模式一致的完整输出。
    let mut other_session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut other_output = CanonicalPoseBatch::new();
    other_session
        .extract_pose_batch(
            FramePlacementToken::new(41),
            &[
                input_lane(7, EDGE_A0, 0),
                input_lane(900, EDGE_A1, 5_000),
                input_lane(3, EDGE_A0, lane_length_mm(&revision, EDGE_A0)),
            ],
            &mut other_output,
        )
        .expect("extract");
    assert_eq!(output, other_output);
    assert_records_bit_identical(output.records(), other_output.records());
}

/// S02：输入重排与非连续记录 ID 时，输出严格保持输入顺序并原样回显身份。
#[test]
fn s02_reordered_inputs_preserve_input_order_and_echo_ids() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let inputs = [
        input_lane(901, EDGE_A1, 2_500),
        input_lane(4, EDGE_A0, 6_000),
        input_lane(70_000, EDGE_A1, 7_500),
        input_lane(1, EDGE_A0, 1),
    ];
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(FramePlacementToken::new(2), &inputs, &mut output)
        .expect("extract");
    assert_eq!(
        output
            .records()
            .iter()
            .map(|record| record.record())
            .map(PoseRecordId::raw)
            .collect::<Vec<_>>(),
        vec![901, 4, 70_000, 1]
    );
    // 单独按另一顺序提取，逐条 pose 与相同 (edge, progress) 的记录位模式一致。
    let reordered = [inputs[1], inputs[3], inputs[0], inputs[2]];
    let mut reordered_output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(3),
            &reordered,
            &mut reordered_output,
        )
        .expect("extract reordered");
    for (record, source) in output.records().iter().zip(inputs.iter()) {
        assert_eq!(record.record(), source.record());
        let expected = reordered_output
            .records()
            .iter()
            .find(|candidate| candidate.record() == source.record())
            .expect("reordered record");
        assert_eq!(record.pose(), expected.pose());
    }
}

/// S03：首条、中间条、末条采样失败时返回具体错误且完整旧输出不变。
#[test]
fn s03_failure_at_first_middle_last_preserves_full_output() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");

    // 先填入与失败批次明显不同的旧内容。
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(10),
            &[input_lane(500, EDGE_A0, 100), input_lane(501, EDGE_A0, 200)],
            &mut output,
        )
        .expect("old batch");

    let length = lane_length_mm(&revision, EDGE_A0);
    for (bad_index, case) in [(0_usize, "first"), (1, "middle"), (2, "last")] {
        let mut inputs = [
            input_lane(600, EDGE_A0, 1_000),
            input_lane(601, EDGE_A0, 2_000),
            input_lane(602, EDGE_A0, 3_000),
        ];
        inputs[bad_index] = input_lane(600 + bad_index as u32, EDGE_A0, length + 1);
        let before = output.clone();
        let error = session
            .extract_pose_batch(
                FramePlacementToken::new(20 + bad_index as u64),
                &inputs,
                &mut output,
            )
            .expect_err("batch must fail");
        match error {
            SpatialError::SharedPoseRecordFailed {
                input_index,
                record,
                source,
            } => {
                assert_eq!(input_index, bad_index, "{case} failing input index");
                assert_eq!(record, inputs[bad_index].record(), "{case} record id");
                assert_eq!(
                    *source,
                    SpatialError::SharedProgressOutOfRange {
                        edge: EDGE_A0,
                        progress_mm: length + 1,
                        length_mm: length,
                    },
                    "{case} source error"
                );
            }
            other => panic!("{case}: unexpected error {other:?}"),
        }
        assert_eq!(output, before, "{case} full output must be unchanged");
    }
}

/// S03 补充：默认（尚未填充的）output 上失败时，完整保持默认 header 与空记录。
#[test]
fn s03_failure_on_default_output_preserves_default_state() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();
    let default_state = CanonicalPoseBatch::new();
    let error = session
        .extract_pose_batch(
            FramePlacementToken::new(30),
            &[input_lane(1, EDGE_A0, 0), input_lane(2, EDGE_UNKNOWN, 0)],
            &mut output,
        )
        .expect_err("batch must fail");
    match error {
        SpatialError::SharedPoseRecordFailed {
            input_index,
            record,
            ..
        } => {
            assert_eq!(input_index, 1);
            assert_eq!(record, PoseRecordId::new(2));
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(output, default_state, "default output must stay untouched");
    assert_eq!(output.network_revision(), None);
    assert_eq!(output.canonical_frame(), None);
    assert_eq!(output.placement_token(), FramePlacementToken::new(0));
    assert!(output.records().is_empty());
}

/// S04：多条输入同时存在错误时，返回现有顺序下的第一个错误。
#[test]
fn s04_multiple_bad_inputs_report_first_error_in_input_order() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[input_lane(10, EDGE_A0, 0)],
            &mut output,
        )
        .expect("old batch");
    let before = output.clone();

    let error = session
        .extract_pose_batch(
            FramePlacementToken::new(2),
            &[
                input_lane(11, EDGE_UNKNOWN, 0),
                input_lane(12, EDGE_A0, u32::MAX),
            ],
            &mut output,
        )
        .expect_err("batch must fail");
    match error {
        SpatialError::SharedPoseRecordFailed {
            input_index,
            record,
            source,
        } => {
            assert_eq!(input_index, 0);
            assert_eq!(record, PoseRecordId::new(11));
            assert_eq!(
                *source,
                SpatialError::UnknownLaneEdge { edge: EDGE_UNKNOWN }
            );
        }
        other => panic!("unexpected error {other:?}"),
    }
    assert_eq!(output, before);
}

/// S05：同一批混用 canonical frame 时返回原 frame mismatch 错误，旧输出全部保留。
#[test]
fn s05_mixed_canonical_frames_reject_whole_batch_and_keep_old_output() {
    let revision = revision();
    let frame_a = frame_of(&revision, EDGE_A0);
    let frame_b = frame_of(&revision, EDGE_B0);
    assert_ne!(frame_a, frame_b, "fixture must provide two frames");
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");

    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(77),
            &[input_lane(31, EDGE_A0, 100), input_lane(32, EDGE_A1, 100)],
            &mut output,
        )
        .expect("old batch");
    let before = output.clone();

    let error = session
        .extract_pose_batch(
            FramePlacementToken::new(78),
            &[
                input_lane(33, EDGE_A0, 1_000),
                input_lane(34, EDGE_A1, 2_000),
                input_lane(35, EDGE_B0, 3_000),
            ],
            &mut output,
        )
        .expect_err("batch must fail");
    assert_eq!(
        error,
        SpatialError::BatchFrameMismatch {
            expected_frame: frame_a,
            actual_frame: frame_b,
        }
    );
    assert_eq!(output, before);
}

/// S06：默认 output 接收空批次时成功且元数据更新、记录为空、frame 为 None。
#[test]
fn s06_default_output_accepts_empty_batch() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(FramePlacementToken::new(5), &[], &mut output)
        .expect("empty batch");
    assert_eq!(output.network_revision(), Some(session.network_revision()));
    assert_eq!(output.canonical_frame(), None);
    assert_eq!(output.placement_token(), FramePlacementToken::new(5));
    assert!(output.records().is_empty());
}

/// S07：有内容的旧 output 接收空批次时记录被替换为空，元数据更新而非提前返回。
#[test]
fn s07_filled_output_accepts_empty_batch() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[input_lane(80, EDGE_A0, 1_000)],
            &mut output,
        )
        .expect("old batch");
    assert_eq!(output.records().len(), 1);

    session
        .extract_pose_batch(FramePlacementToken::new(2), &[], &mut output)
        .expect("empty batch");
    assert_eq!(output.network_revision(), Some(session.network_revision()));
    assert_eq!(output.canonical_frame(), None);
    assert_eq!(output.placement_token(), FramePlacementToken::new(2));
    assert!(output.records().is_empty());
}

/// S08：失败后修正输入重试成功，结果与干净执行位模式一致。
#[test]
fn s08_retry_after_failure_matches_clean_extraction() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let good = [
        input_lane(91, EDGE_A0, 3_333),
        input_lane(92, EDGE_A1, 4_444),
        input_lane(93, EDGE_A0, 5_555),
    ];
    let mut bad = good;
    bad[1] = input_lane(92, EDGE_A1, u32::MAX);

    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[input_lane(1, EDGE_A0, 0)],
            &mut output,
        )
        .expect("warm-up batch");
    session
        .extract_pose_batch(FramePlacementToken::new(2), &bad, &mut output)
        .expect_err("batch must fail");
    session
        .extract_pose_batch(FramePlacementToken::new(3), &good, &mut output)
        .expect("retry");

    let mut clean_session = SpatialSession::bind(revision)
        .expect("bind")
        .expect("session");
    let mut clean_output = CanonicalPoseBatch::new();
    clean_session
        .extract_pose_batch(FramePlacementToken::new(3), &good, &mut clean_output)
        .expect("clean");
    assert_eq!(output, clean_output);
    assert_records_bit_identical(output.records(), clean_output.records());
}

/// S09：连续成功、失败、空批、再次成功的每次状态转换都正确。
#[test]
fn s09_success_failure_empty_success_sequence() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut output = CanonicalPoseBatch::new();

    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[input_lane(1, EDGE_A0, 10), input_lane(2, EDGE_A0, 20)],
            &mut output,
        )
        .expect("first success");
    let first = output.clone();
    assert_eq!(output.records().len(), 2);

    let error = session
        .extract_pose_batch(
            FramePlacementToken::new(2),
            &[input_lane(3, EDGE_UNKNOWN, 0)],
            &mut output,
        )
        .expect_err("failure");
    assert!(matches!(
        error,
        SpatialError::SharedPoseRecordFailed { input_index: 0, .. }
    ));
    assert_eq!(output, first);

    session
        .extract_pose_batch(FramePlacementToken::new(3), &[], &mut output)
        .expect("empty batch");
    assert_eq!(output.records().len(), 0);
    assert_eq!(output.placement_token(), FramePlacementToken::new(3));
    assert_eq!(output.canonical_frame(), None);
    let empty = output.clone();

    session
        .extract_pose_batch(
            FramePlacementToken::new(4),
            &[input_lane(4, EDGE_A1, 40), input_lane(5, EDGE_A1, 50)],
            &mut output,
        )
        .expect("second success");
    assert_eq!(
        output
            .records()
            .iter()
            .map(|record| record.record())
            .map(PoseRecordId::raw)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert_eq!(output.placement_token(), FramePlacementToken::new(4));
    assert_ne!(output, empty);
    assert_ne!(output, first);
}

/// S10：两个 output 交替使用时，更新一方不修改另一方的已提交内容。
#[test]
fn s10_alternating_outputs_stay_independent() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let mut a = CanonicalPoseBatch::new();
    let mut b = CanonicalPoseBatch::new();

    session
        .extract_pose_batch(
            FramePlacementToken::new(1),
            &[input_lane(1, EDGE_A0, 100), input_lane(2, EDGE_A0, 200)],
            &mut a,
        )
        .expect("fill a");
    session
        .extract_pose_batch(
            FramePlacementToken::new(2),
            &[input_lane(3, EDGE_A1, 300)],
            &mut b,
        )
        .expect("fill b");
    let a_snapshot = a.clone();
    let b_snapshot = b.clone();

    session
        .extract_pose_batch(
            FramePlacementToken::new(3),
            &[input_lane(9, EDGE_A0, 400), input_lane(8, EDGE_A0, 500)],
            &mut a,
        )
        .expect("update a");
    assert_eq!(b, b_snapshot, "updating a must not modify b");
    assert_eq!(a.placement_token(), FramePlacementToken::new(3));
    let a_after_update = a.clone();

    session
        .extract_pose_batch(
            FramePlacementToken::new(4),
            &[input_lane(7, EDGE_A1, 600)],
            &mut b,
        )
        .expect("update b");
    assert_eq!(a, a_after_update, "updating b must not modify committed a");
    assert_eq!(a.placement_token(), FramePlacementToken::new(3));
    assert_eq!(b.placement_token(), FramePlacementToken::new(4));
    assert_eq!(
        a.records()
            .iter()
            .map(|record| record.record())
            .map(PoseRecordId::raw)
            .collect::<Vec<_>>(),
        vec![9, 8]
    );
    assert_ne!(a, a_snapshot);
    assert_ne!(b, b_snapshot);
}

/// S11：合法 Lane 与 Parking 输入经同一提交机制产生一致结果。
#[test]
fn s11_lane_and_parking_inputs_commit_through_same_path() {
    let revision = revision();
    let mut session = SpatialSession::bind(Arc::clone(&revision))
        .expect("bind")
        .expect("session");
    let inputs = [
        PoseInput::parking(PoseRecordId::new(21), PARKING0),
        input_lane(22, EDGE_A0, 4_000),
        PoseInput::parking(PoseRecordId::new(23), PARKING0),
        input_lane(24, EDGE_A1, 8_000),
    ];
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(FramePlacementToken::new(6), &inputs, &mut output)
        .expect("extract");
    assert_eq!(
        output
            .records()
            .iter()
            .map(|record| record.record())
            .map(PoseRecordId::raw)
            .collect::<Vec<_>>(),
        vec![21, 22, 23, 24]
    );
    assert_eq!(output.canonical_frame(), Some(frame_of(&revision, EDGE_A0)));

    // 同一输入重复提取位模式一致；停车位姿与车道位姿经同一批合同输出。
    let mut repeat = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(FramePlacementToken::new(7), &inputs, &mut repeat)
        .expect("repeat");
    assert_records_bit_identical(output.records(), repeat.records());
    let parking_pose = output.records()[0].pose();
    let same_parking_pose = output.records()[2].pose();
    assert_eq!(parking_pose, same_parking_pose);
}
